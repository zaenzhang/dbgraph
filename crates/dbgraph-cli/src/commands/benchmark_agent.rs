//! Benchmark CLI command implementations.

use std::fmt::Write as FmtWrite;
use std::fs;
use std::path::Path;

use dbgraph_core::benchmark::{synthetic_schema_snapshot, SyntheticSchemaOptions};
use dbgraph_core::{DbGraphError, Result};
use dbgraph_graph::analysis::{AnalysisAnalyzer, AnalysisOptions};
use dbgraph_graph::context::{ContextBuilder, ContextOptions, RankingWeights};
use serde::Serialize;

use crate::commands::common::{discover_context, latest_snapshot, require_graph_index};
use crate::AgentBenchmarkFormat;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BenchmarkReport {
    tables: usize,
    columns_per_table: usize,
    object_count: usize,
    edge_count: usize,
    schema_hash: String,
}

#[derive(Debug, Clone)]
pub(crate) struct AgentBenchmarkOptions {
    pub(crate) scenario: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentBenchmarkReport {
    pub(crate) scenario: String,
    pub(crate) summary: AgentBenchmarkSummary,
    pub(crate) cases: Vec<AgentBenchmarkCaseReport>,
    pub(crate) limitations: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentBenchmarkSummary {
    pub(crate) baseline_estimated_tokens: usize,
    pub(crate) dbgraph_estimated_tokens: usize,
    pub(crate) token_reduction_percent: f64,
    pub(crate) baseline_retrieval_steps: usize,
    pub(crate) dbgraph_retrieval_steps: usize,
    pub(crate) evidence_recall_delta: f64,
    pub(crate) precision_delta: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentBenchmarkCaseReport {
    pub(crate) id: String,
    pub(crate) question: String,
    pub(crate) expected_objects: Vec<String>,
    pub(crate) baseline: AgentBenchmarkModeMetrics,
    pub(crate) dbgraph: AgentBenchmarkModeMetrics,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentBenchmarkModeMetrics {
    pub(crate) context_bytes: usize,
    pub(crate) estimated_tokens: usize,
    pub(crate) retrieval_steps: usize,
    pub(crate) evidence_recall: f64,
    pub(crate) relevant_object_precision: f64,
    pub(crate) matched_objects: Vec<String>,
}

struct AgentBenchmarkCase {
    id: &'static str,
    question: &'static str,
    expected_objects: &'static [&'static str],
    query: &'static str,
}

pub(crate) fn benchmark_project(
    tables: usize,
    columns_per_table: usize,
) -> Result<BenchmarkReport> {
    let snapshot = synthetic_schema_snapshot(SyntheticSchemaOptions {
        table_count: tables,
        columns_per_table,
    });
    let schema_hash = dbgraph_core::snapshot::compute_schema_hash(&snapshot)?;
    Ok(BenchmarkReport {
        tables,
        columns_per_table,
        object_count: snapshot.objects.len(),
        edge_count: snapshot.edges.len(),
        schema_hash,
    })
}

pub(crate) fn benchmark_agent_project(
    start: impl AsRef<Path>,
    options: AgentBenchmarkOptions,
) -> Result<AgentBenchmarkReport> {
    if options.scenario != "teashop" {
        return Err(DbGraphError::invalid_argument(
            "`benchmark-agent` currently supports only `--scenario teashop`",
        ));
    }
    let context = discover_context(start.as_ref())?;
    require_graph_index(&context)?;
    let snapshot = latest_snapshot(&context)?;
    let analysis = AnalysisAnalyzer::new().analyze(&snapshot, &AnalysisOptions::default());
    let baseline_materials = collect_baseline_materials(context.project_root())?;
    let raw_catalog =
        serde_json::to_string_pretty(&snapshot).map_err(|source| DbGraphError::Internal {
            message: format!("failed to serialize raw catalog for benchmark: {source}"),
        })?;
    let baseline_context = format!(
        "{}\n\n-- raw database catalog --\n{raw_catalog}",
        baseline_materials.join("\n\n")
    );
    let analysis_json =
        serde_json::to_string(&analysis).map_err(|source| DbGraphError::Internal {
            message: format!("failed to serialize analysis for benchmark: {source}"),
        })?;
    let cases = teashop_benchmark_cases()
        .into_iter()
        .map(|case| {
            let context_package = ContextBuilder::new(RankingWeights::default()).build(
                &snapshot,
                case.query,
                &ContextOptions {
                    token_budget: 1_200,
                    max_objects: 12,
                },
            );
            let context_json = serde_json::to_string(&context_package).map_err(|source| {
                DbGraphError::Internal {
                    message: format!("failed to serialize context for benchmark: {source}"),
                }
            })?;
            let relevant_findings = analysis
                .findings
                .iter()
                .filter(|finding| {
                    case.id == "review-report"
                        || case.expected_objects.iter().any(|object| {
                            finding.object.contains(object) || object.contains(&finding.object)
                        })
                })
                .cloned()
                .collect::<Vec<_>>();
            let findings_json = serde_json::to_string(&relevant_findings).map_err(|source| {
                DbGraphError::Internal {
                    message: format!("failed to serialize findings for benchmark: {source}"),
                }
            })?;
            let dbgraph_context = if case.id == "review-report" {
                format!("{analysis_json}\n{context_json}")
            } else {
                format!("{findings_json}\n{context_json}")
            };
            Ok(AgentBenchmarkCaseReport {
                id: case.id.to_owned(),
                question: case.question.to_owned(),
                expected_objects: case
                    .expected_objects
                    .iter()
                    .map(|object| (*object).to_owned())
                    .collect(),
                baseline: benchmark_mode_metrics(
                    &baseline_context,
                    baseline_materials.len().max(1),
                    case.expected_objects,
                ),
                dbgraph: benchmark_mode_metrics(&dbgraph_context, 2, case.expected_objects),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let summary = summarize_agent_benchmark(&cases);
    Ok(AgentBenchmarkReport {
        scenario: options.scenario,
        summary,
        cases,
        limitations: vec![
            "Offline benchmark estimates context quality; it does not call an LLM.".to_owned(),
            "Token counts are rough deterministic estimates, not provider billing numbers."
                .to_owned(),
        ],
    })
}

fn teashop_benchmark_cases() -> Vec<AgentBenchmarkCase> {
    vec![
        AgentBenchmarkCase {
            id: "pii-fields",
            question: "Which columns are likely PII or secrets?",
            expected_objects: &["public.customers.email", "public.payments.provider_token"],
            query: "PII sensitive email provider token",
        },
        AgentBenchmarkCase {
            id: "sql-sensitive-read",
            question: "Which SQL artifacts read sensitive customer fields?",
            expected_objects: &["public.customers.email"],
            query: "SQL reads customers email sensitive column",
        },
        AgentBenchmarkCase {
            id: "orders-status-quality",
            question: "What risks exist around public.orders.status?",
            expected_objects: &["public.orders.status"],
            query: "orders status quality performance",
        },
        AgentBenchmarkCase {
            id: "schema-quality",
            question: "Which schema quality issues need review?",
            expected_objects: &["public.orders", "public.payments"],
            query: "missing primary key foreign key schema quality",
        },
        AgentBenchmarkCase {
            id: "index-risk",
            question: "Which workload columns may need indexes?",
            expected_objects: &["public.orders.status"],
            query: "filter join without index orders status",
        },
        AgentBenchmarkCase {
            id: "review-report",
            question: "Can the agent produce a structured database review report?",
            expected_objects: &[
                "Security & Privacy",
                "Data Integrity & Schema Quality",
                "Performance",
            ],
            query: "structured analysis report security quality performance",
        },
    ]
}

fn collect_baseline_materials(root: &Path) -> Result<Vec<String>> {
    let mut materials = Vec::new();
    collect_baseline_materials_from(root, root, &mut materials)?;
    Ok(materials)
}

fn collect_baseline_materials_from(
    root: &Path,
    dir: &Path,
    materials: &mut Vec<String>,
) -> Result<()> {
    if !dir.is_dir() {
        return Ok(());
    }
    for entry in fs::read_dir(dir).map_err(|source| DbGraphError::io(dir, source))? {
        let entry = entry.map_err(|source| DbGraphError::io(dir, source))?;
        let path = entry.path();
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        if name == ".dbgraph" || name == "target" || name == ".git" {
            continue;
        }
        if path.is_dir() {
            collect_baseline_materials_from(root, &path, materials)?;
        } else if is_baseline_material(&path) {
            let relative = path.strip_prefix(root).unwrap_or(&path).display();
            let content = fs::read_to_string(&path).unwrap_or_default();
            materials.push(format!("-- {relative} --\n{content}"));
        }
    }
    Ok(())
}

fn is_baseline_material(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| matches!(extension, "sql" | "md" | "json" | "toml"))
}

fn benchmark_mode_metrics(
    context: &str,
    retrieval_steps: usize,
    expected_objects: &[&str],
) -> AgentBenchmarkModeMetrics {
    let matched_objects = expected_objects
        .iter()
        .filter(|object| context.contains(**object))
        .map(|object| (*object).to_owned())
        .collect::<Vec<_>>();
    let evidence_recall = ratio(matched_objects.len(), expected_objects.len());
    let relevant_object_precision = if matched_objects.is_empty() {
        0.0
    } else {
        ratio(
            matched_objects.len(),
            count_object_like_mentions(context).max(matched_objects.len()),
        )
    };
    AgentBenchmarkModeMetrics {
        context_bytes: context.len(),
        estimated_tokens: estimate_tokens(context),
        retrieval_steps,
        evidence_recall,
        relevant_object_precision,
        matched_objects,
    }
}

fn count_object_like_mentions(context: &str) -> usize {
    context
        .split(|ch: char| ch.is_whitespace() || matches!(ch, ',' | '"' | '\'' | '(' | ')' | ';'))
        .filter(|token| token.matches('.').count() >= 1 && token.contains("public."))
        .count()
}

fn ratio(numerator: usize, denominator: usize) -> f64 {
    if denominator == 0 {
        0.0
    } else {
        usize_to_f64(numerator) / usize_to_f64(denominator)
    }
}

fn usize_to_f64(value: usize) -> f64 {
    f64::from(u32::try_from(value).unwrap_or(u32::MAX))
}

fn estimate_tokens(text: &str) -> usize {
    text.chars().count().div_ceil(4).max(1)
}

fn summarize_agent_benchmark(cases: &[AgentBenchmarkCaseReport]) -> AgentBenchmarkSummary {
    let baseline_estimated_tokens = cases
        .iter()
        .map(|case| case.baseline.estimated_tokens)
        .sum::<usize>();
    let dbgraph_estimated_tokens = cases
        .iter()
        .map(|case| case.dbgraph.estimated_tokens)
        .sum::<usize>();
    let baseline_retrieval_steps = cases
        .iter()
        .map(|case| case.baseline.retrieval_steps)
        .sum::<usize>();
    let dbgraph_retrieval_steps = cases
        .iter()
        .map(|case| case.dbgraph.retrieval_steps)
        .sum::<usize>();
    let baseline_recall = average(cases.iter().map(|case| case.baseline.evidence_recall));
    let dbgraph_recall = average(cases.iter().map(|case| case.dbgraph.evidence_recall));
    let baseline_precision = average(
        cases
            .iter()
            .map(|case| case.baseline.relevant_object_precision),
    );
    let dbgraph_precision = average(
        cases
            .iter()
            .map(|case| case.dbgraph.relevant_object_precision),
    );
    AgentBenchmarkSummary {
        baseline_estimated_tokens,
        dbgraph_estimated_tokens,
        token_reduction_percent: if baseline_estimated_tokens == 0 {
            0.0
        } else {
            100.0 * usize_to_f64(baseline_estimated_tokens.saturating_sub(dbgraph_estimated_tokens))
                / usize_to_f64(baseline_estimated_tokens)
        },
        baseline_retrieval_steps,
        dbgraph_retrieval_steps,
        evidence_recall_delta: dbgraph_recall - baseline_recall,
        precision_delta: dbgraph_precision - baseline_precision,
    }
}

fn average(values: impl Iterator<Item = f64>) -> f64 {
    let values = values.collect::<Vec<_>>();
    if values.is_empty() {
        0.0
    } else {
        values.iter().sum::<f64>() / usize_to_f64(values.len())
    }
}

pub(crate) fn print_benchmark_report(report: &BenchmarkReport) {
    println!("DbGraph benchmark schema");
    println!("Tables: {}", report.tables);
    println!("Columns per table: {}", report.columns_per_table);
    println!("Objects: {}", report.object_count);
    println!("Edges: {}", report.edge_count);
    println!("Schema hash: {}", report.schema_hash);
}

pub(crate) fn write_agent_benchmark_output(
    report: &AgentBenchmarkReport,
    format: AgentBenchmarkFormat,
    output: Option<&Path>,
) -> Result<()> {
    let rendered = match format {
        AgentBenchmarkFormat::Text => render_agent_benchmark_text(report),
        AgentBenchmarkFormat::Json => {
            serde_json::to_string_pretty(report).map_err(|source| DbGraphError::Internal {
                message: format!("failed to serialize agent benchmark: {source}"),
            })?
        }
        AgentBenchmarkFormat::Markdown => render_agent_benchmark_markdown(report),
    };
    if let Some(path) = output {
        fs::write(path, format!("{rendered}\n")).map_err(|source| DbGraphError::io(path, source))
    } else {
        println!("{rendered}");
        Ok(())
    }
}

fn render_agent_benchmark_text(report: &AgentBenchmarkReport) -> String {
    let mut output = String::new();
    let _ = writeln!(output, "DbGraph offline agent benchmark");
    let _ = writeln!(output, "Scenario: {}", report.scenario);
    let _ = writeln!(
        output,
        "Estimated tokens: baseline={} dbgraph={} reduction={:.1}%",
        report.summary.baseline_estimated_tokens,
        report.summary.dbgraph_estimated_tokens,
        report.summary.token_reduction_percent
    );
    let _ = writeln!(
        output,
        "Retrieval steps: baseline={} dbgraph={}",
        report.summary.baseline_retrieval_steps, report.summary.dbgraph_retrieval_steps
    );
    for case in &report.cases {
        let _ = writeln!(
            output,
            "- {}: baseline recall {:.2}, dbgraph recall {:.2}",
            case.id, case.baseline.evidence_recall, case.dbgraph.evidence_recall
        );
    }
    output
}

fn render_agent_benchmark_markdown(report: &AgentBenchmarkReport) -> String {
    let mut output = String::new();
    output.push_str("# DbGraph Offline Agent Benchmark\n\n");
    let _ = writeln!(output, "- Scenario: `{}`", report.scenario);
    let _ = writeln!(
        output,
        "- Token reduction: `{:.1}%`\n",
        report.summary.token_reduction_percent
    );
    output.push_str("| Metric | Baseline | DbGraph |\n|---|---:|---:|\n");
    let _ = writeln!(
        output,
        "| Estimated tokens | {} | {} |",
        report.summary.baseline_estimated_tokens, report.summary.dbgraph_estimated_tokens
    );
    let _ = writeln!(
        output,
        "| Retrieval steps | {} | {} |",
        report.summary.baseline_retrieval_steps, report.summary.dbgraph_retrieval_steps
    );
    let _ = writeln!(
        output,
        "| Evidence recall delta | - | {:+.2} |",
        report.summary.evidence_recall_delta
    );
    let _ = writeln!(
        output,
        "| Precision delta | - | {:+.2} |\n",
        report.summary.precision_delta
    );
    output.push_str("## Cases\n\n");
    for case in &report.cases {
        let _ = writeln!(output, "### {}\n", case.id);
        let _ = writeln!(output, "{}\n", case.question);
        let _ = writeln!(
            output,
            "Expected objects: `{}`\n",
            case.expected_objects.join("`, `")
        );
        output.push_str(
            "| Mode | Tokens | Steps | Recall | Precision |\n|---|---:|---:|---:|---:|\n",
        );
        let _ = writeln!(
            output,
            "| Baseline | {} | {} | {:.2} | {:.2} |",
            case.baseline.estimated_tokens,
            case.baseline.retrieval_steps,
            case.baseline.evidence_recall,
            case.baseline.relevant_object_precision
        );
        let _ = writeln!(
            output,
            "| DbGraph | {} | {} | {:.2} | {:.2} |\n",
            case.dbgraph.estimated_tokens,
            case.dbgraph.retrieval_steps,
            case.dbgraph.evidence_recall,
            case.dbgraph.relevant_object_precision
        );
    }
    output.push_str("## Limitations\n\n");
    for limitation in &report.limitations {
        let _ = writeln!(output, "- {limitation}");
    }
    output
}
