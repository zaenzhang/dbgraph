//! Analysis report section and score helpers.

use std::collections::BTreeMap;

use super::{AnalysisFinding, AnalysisScope, AnalysisSection, FindingSeverity};

pub(super) fn risk_score(findings: &[AnalysisFinding]) -> u32 {
    findings
        .iter()
        .map(|finding| match finding.severity {
            FindingSeverity::Critical => 25,
            FindingSeverity::High => 10,
            FindingSeverity::Medium => 5,
            FindingSeverity::Low => 1,
        })
        .sum()
}

pub(super) fn overview_summary(total_findings: usize, risk_score: u32) -> String {
    if total_findings == 0 {
        "No risk, quality, or performance findings were detected.".to_owned()
    } else {
        format!("{total_findings} findings detected with risk score {risk_score}.")
    }
}

pub(super) fn build_sections(findings: &[AnalysisFinding]) -> Vec<AnalysisSection> {
    [
        (
            "security_privacy",
            "Security & Privacy",
            "sensitive data exposure and risky SQL access patterns.",
        ),
        (
            "data_integrity_schema_quality",
            "Data Integrity & Schema Quality",
            "Schema constraints and relationship quality issues.",
        ),
        (
            "sql_workload_safety",
            "SQL Workload & Safety",
            "SQL write patterns that can affect more rows than intended.",
        ),
        (
            "performance",
            "Performance",
            "Query workload patterns that may need supporting indexes.",
        ),
        (
            "data_profiling_business_rules",
            "Data Profiling & Business Rules",
            "Findings derived from explicitly allowlisted business-row samples.",
        ),
    ]
    .into_iter()
    .map(|(id, title, summary)| {
        let section_findings = findings
            .iter()
            .filter(|finding| finding_belongs_to_section(finding, id));
        let mut severity_counts = BTreeMap::new();
        let mut finding_count = 0;
        for finding in section_findings {
            finding_count += 1;
            *severity_counts
                .entry(finding.severity.label().to_owned())
                .or_insert(0) += 1;
        }
        AnalysisSection {
            id: id.to_owned(),
            title: title.to_owned(),
            summary: summary.to_owned(),
            finding_count,
            severity_counts,
        }
    })
    .collect()
}

fn finding_belongs_to_section(finding: &AnalysisFinding, section_id: &str) -> bool {
    match section_id {
        "security_privacy" => {
            finding.scope == AnalysisScope::Risk
                && (finding.rule_id.contains("sensitive") || finding.rule_id == "risk.select_star")
        }
        "data_integrity_schema_quality" => {
            finding.scope == AnalysisScope::Quality && !finding.rule_id.starts_with("data.")
        }
        "sql_workload_safety" => matches!(
            finding.rule_id.as_str(),
            "risk.update_without_where" | "risk.delete_without_where"
        ),
        "performance" => finding.scope == AnalysisScope::Performance,
        "data_profiling_business_rules" => finding.rule_id.starts_with("data."),
        _ => false,
    }
}
