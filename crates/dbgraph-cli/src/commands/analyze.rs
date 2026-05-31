//! Analysis CLI command implementation.

use std::fmt::Write as FmtWrite;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use dbgraph_core::project::ProjectContext;
use dbgraph_core::{DbGraphError, Result};
use dbgraph_graph::analysis::{
    AnalysisAnalyzer, AnalysisFinding, AnalysisGate, AnalysisOptions, AnalysisReport,
    AnalysisScope, FindingSeverity,
};
use serde::{Deserialize, Serialize};

use crate::commands::common::{discover_context, latest_snapshot, require_graph_index};
use crate::AnalysisOutputFormat;

#[derive(Debug, Clone)]
pub(crate) struct AnalysisCliOptions {
    pub(crate) scope: AnalysisScope,
    pub(crate) include_suppressed: bool,
    pub(crate) suppressions: Option<PathBuf>,
    pub(crate) fail_on: Option<FindingSeverity>,
    pub(crate) fail_on_new: Option<FindingSeverity>,
    pub(crate) baseline: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SuppressionFile {
    version: u32,
    suppressions: Vec<FindingSuppression>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FindingSuppression {
    rule_id: String,
    object: String,
    reason: String,
    owner: String,
    expires_at: Option<String>,
}

#[cfg(test)]
pub(crate) fn analyze_project(
    start: impl AsRef<Path>,
    scope: AnalysisScope,
) -> Result<AnalysisReport> {
    analyze_project_with_options(
        start,
        &AnalysisCliOptions {
            scope,
            include_suppressed: false,
            suppressions: None,
            fail_on: None,
            fail_on_new: None,
            baseline: None,
        },
    )
}

pub(crate) fn analyze_project_with_options(
    start: impl AsRef<Path>,
    options: &AnalysisCliOptions,
) -> Result<AnalysisReport> {
    let context = discover_context(start.as_ref())?;
    require_graph_index(&context)?;
    let snapshot = latest_snapshot(&context)?;
    let mut report = AnalysisAnalyzer::new().analyze(
        &snapshot,
        &AnalysisOptions {
            scope: options.scope,
        },
    );
    apply_suppressions(&context, &mut report, options)?;
    apply_analysis_gate(&mut report, options)?;
    Ok(report)
}

fn apply_suppressions(
    context: &ProjectContext,
    report: &mut AnalysisReport,
    options: &AnalysisCliOptions,
) -> Result<()> {
    let suppression_path = options
        .suppressions
        .clone()
        .unwrap_or_else(|| context.dbgraph_dir().join("suppressions.json"));
    if !suppression_path.is_file() {
        return Ok(());
    }
    let policy_text = fs::read_to_string(&suppression_path)
        .map_err(|source| DbGraphError::io(&suppression_path, source))?;
    let policy = serde_json::from_str::<SuppressionFile>(&policy_text).map_err(|source| {
        DbGraphError::invalid_config(format!(
            "failed to parse suppressions {}: {source}",
            suppression_path.display()
        ))
    })?;
    if policy.version != 1 {
        return Err(DbGraphError::invalid_config(
            "suppressions.json version must be 1",
        ));
    }
    let mut active = Vec::new();
    let mut suppressed = Vec::new();
    let mut counts = std::collections::BTreeMap::new();
    for finding in std::mem::take(&mut report.findings) {
        let matched = policy.suppressions.iter().find(|suppression| {
            suppression.rule_id == finding.rule_id && suppression.object == finding.object
        });
        if let Some(suppression) = matched {
            if suppression
                .expires_at
                .as_deref()
                .is_some_and(suppression_expired)
            {
                *counts.entry("expired".to_owned()).or_insert(0) += 1;
                active.push(finding);
            } else {
                *counts.entry("suppressed".to_owned()).or_insert(0) += 1;
                suppressed.push(finding);
            }
        } else {
            active.push(finding);
        }
    }
    if options.include_suppressed {
        let mut combined = active.clone();
        combined.extend(suppressed.clone());
        combined.sort_by(|left, right| {
            right
                .severity
                .cmp(&left.severity)
                .then_with(|| left.rule_id.cmp(&right.rule_id))
                .then_with(|| left.object.cmp(&right.object))
        });
        report.findings = combined;
    } else {
        report.findings = active;
    }
    report.suppressed_findings = suppressed;
    report.suppression_counts = counts;
    refresh_analysis_summary(report);
    Ok(())
}

fn suppression_expired(value: &str) -> bool {
    parse_yyyy_mm_dd(value).is_some_and(|date| date < today_utc_date())
}

fn parse_yyyy_mm_dd(value: &str) -> Option<(i32, u32, u32)> {
    let mut parts = value.split('-');
    let year = parts.next()?.parse().ok()?;
    let month = parts.next()?.parse().ok()?;
    let day = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((year, month, day))
}

fn today_utc_date() -> (i32, u32, u32) {
    let days = UNIX_EPOCH.elapsed().map_or(0, |duration| {
        i64::try_from(duration.as_secs() / 86_400).unwrap_or(0)
    });
    civil_from_days(days)
}

fn civil_from_days(days_since_epoch: i64) -> (i32, u32, u32) {
    let z = days_since_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    let year = y + i64::from(month <= 2);
    (
        i32::try_from(year).unwrap_or(i32::MAX),
        u32::try_from(month).unwrap_or(1),
        u32::try_from(day).unwrap_or(1),
    )
}

fn refresh_analysis_summary(report: &mut AnalysisReport) {
    report.severity_counts.clear();
    for finding in &report.findings {
        *report
            .severity_counts
            .entry(finding.severity.label().to_owned())
            .or_insert(0) += 1;
    }
    report.risk_score = report
        .findings
        .iter()
        .map(|finding| match finding.severity {
            FindingSeverity::Critical => 25,
            FindingSeverity::High => 10,
            FindingSeverity::Medium => 5,
            FindingSeverity::Low => 1,
        })
        .sum();
    report.overview.total_findings = report.findings.len();
    report.overview.risk_score = report.risk_score;
    report.overview.summary = if report.findings.is_empty() {
        "No active risk, quality, or performance findings were detected.".to_owned()
    } else {
        format!(
            "{} active findings detected with risk score {}.",
            report.findings.len(),
            report.risk_score
        )
    };
    report.top_findings = report.findings.iter().take(5).cloned().collect();
}

fn apply_analysis_gate(report: &mut AnalysisReport, options: &AnalysisCliOptions) -> Result<()> {
    if options.fail_on.is_none() && options.fail_on_new.is_none() {
        return Ok(());
    }
    let failed_fingerprints = options.fail_on.map_or_else(Vec::new, |threshold| {
        report
            .findings
            .iter()
            .filter(|finding| finding.severity >= threshold)
            .map(|finding| finding.fingerprint.clone())
            .collect()
    });
    let baseline_fingerprints = options
        .baseline
        .as_deref()
        .map(read_baseline_fingerprints)
        .transpose()?
        .unwrap_or_default();
    let new_failed_fingerprints = options.fail_on_new.map_or_else(Vec::new, |threshold| {
        report
            .findings
            .iter()
            .filter(|finding| finding.severity >= threshold)
            .filter(|finding| !baseline_fingerprints.contains(&finding.fingerprint))
            .map(|finding| finding.fingerprint.clone())
            .collect()
    });
    let passed = failed_fingerprints.is_empty() && new_failed_fingerprints.is_empty();
    report.gate = Some(AnalysisGate {
        passed,
        threshold: options.fail_on.map(|severity| severity.label().to_owned()),
        new_threshold: options
            .fail_on_new
            .map(|severity| severity.label().to_owned()),
        failed_fingerprints,
        new_failed_fingerprints,
        message: if passed {
            "analysis gate passed".to_owned()
        } else {
            "analysis gate failed".to_owned()
        },
    });
    Ok(())
}

fn read_baseline_fingerprints(path: &Path) -> Result<std::collections::BTreeSet<String>> {
    let content = fs::read_to_string(path).map_err(|source| DbGraphError::io(path, source))?;
    let report = serde_json::from_str::<AnalysisReport>(&content).map_err(|source| {
        DbGraphError::invalid_config(format!(
            "failed to parse analysis baseline {}: {source}",
            path.display()
        ))
    })?;
    Ok(report
        .findings
        .into_iter()
        .chain(report.suppressed_findings)
        .map(|finding| finding.fingerprint)
        .collect())
}

pub(crate) fn write_analysis_output(
    report: &AnalysisReport,
    format: AnalysisOutputFormat,
    output: Option<&Path>,
) -> Result<()> {
    let rendered = match format {
        AnalysisOutputFormat::Text => render_analysis_text(report),
        AnalysisOutputFormat::Json => {
            serde_json::to_string_pretty(report).map_err(|source| DbGraphError::Internal {
                message: format!("failed to serialize analysis report: {source}"),
            })?
        }
        AnalysisOutputFormat::Markdown => render_analysis_markdown(report),
    };
    if let Some(path) = output {
        fs::write(path, format!("{rendered}\n")).map_err(|source| DbGraphError::io(path, source))
    } else {
        println!("{rendered}");
        Ok(())
    }
}

fn render_analysis_text(report: &AnalysisReport) -> String {
    let mut output = String::new();
    output.push_str("DbGraph analysis\n");
    let _ = writeln!(output, "Snapshot: {}", report.snapshot_id);
    let _ = writeln!(output, "Scope: {:?}", report.scope);
    let _ = writeln!(output, "Findings: {}", report.findings.len());
    let _ = writeln!(output, "Risk score: {}", report.risk_score);
    let _ = writeln!(output, "Summary: {}", report.overview.summary);
    for section in &report.sections {
        let _ = writeln!(
            output,
            "\n{}: {} findings\n",
            section.title, section.finding_count
        );
    }
    for finding in &report.findings {
        output.push_str(&render_analysis_finding(finding));
    }
    output
}

fn render_analysis_markdown(report: &AnalysisReport) -> String {
    let mut output = String::new();
    output.push_str("# DbGraph Analysis Report\n\n");
    let _ = writeln!(output, "- Snapshot: `{}`", report.snapshot_id);
    let _ = writeln!(output, "- Scope: `{:?}`", report.scope);
    let _ = writeln!(output, "- Findings: `{}`", report.findings.len());
    let _ = writeln!(output, "- Risk score: `{}`\n", report.risk_score);
    let _ = writeln!(output, "{}\n", report.overview.summary);
    output.push_str("## Sections\n\n");
    for section in &report.sections {
        let _ = writeln!(
            output,
            "- **{}**: {} findings. {}\n",
            section.title, section.finding_count, section.summary
        );
    }
    output.push_str("\n## Top Findings\n\n");
    for finding in &report.top_findings {
        output.push_str(&render_analysis_finding_markdown(finding));
    }
    output.push_str("\n## All Findings\n\n");
    for finding in &report.findings {
        output.push_str(&render_analysis_finding_markdown(finding));
    }
    output
}

fn render_analysis_finding(finding: &AnalysisFinding) -> String {
    let mut output = String::new();
    let _ = writeln!(
        output,
        "\n[{:?}] {} {} :: {}\n",
        finding.severity, finding.rule_id, finding.object, finding.message
    );
    let _ = writeln!(output, "  title: {}", finding.title);
    let _ = writeln!(output, "  evidence: {}", finding.evidence);
    let _ = writeln!(output, "  impact: {}", finding.impact);
    let _ = writeln!(output, "  suggested fix: {}", finding.suggested_fix);
    if !finding.related_objects.is_empty() {
        let _ = writeln!(
            output,
            "  related objects: {}\n",
            finding.related_objects.join(", ")
        );
    }
    output
}

fn render_analysis_finding_markdown(finding: &AnalysisFinding) -> String {
    let mut output = String::new();
    let _ = writeln!(output, "### {} `{}`\n", finding.title, finding.object);
    let _ = write!(
        output,
        "- Severity: `{:?}`\n- Rule: `{}`\n- Message: {}\n- Evidence: {}\n- Impact: {}\n- Suggested fix: {}\n- Confidence: `{:.2}`\n",
        finding.severity,
        finding.rule_id,
        finding.message,
        finding.evidence,
        finding.impact,
        finding.suggested_fix,
        finding.confidence,
    );
    if !finding.tags.is_empty() {
        let _ = writeln!(output, "- Tags: `{}`", finding.tags.join("`, `"));
    }
    if !finding.related_objects.is_empty() {
        let _ = writeln!(
            output,
            "- Related objects: `{}`\n",
            finding.related_objects.join("`, `")
        );
    }
    output.push('\n');
    output
}
