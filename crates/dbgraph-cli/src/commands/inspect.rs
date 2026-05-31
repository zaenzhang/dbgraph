//! Inspect, validation, context, diff, and impact CLI commands.

use std::fs;
use std::path::Path;

use dbgraph_core::diff::{DiffEngine, SchemaDiff};
use dbgraph_core::model::{
    ColumnProfile, DbEdge, DbObject, DbObjectKind, DbSnapshot, TableProfile,
};
use dbgraph_core::project::ProjectContext;
use dbgraph_core::snapshot::SnapshotStore;
use dbgraph_core::{DbGraphError, Result};
use dbgraph_graph::context::{ContextBuilder, ContextOptions, ContextPackage, RankingWeights};
use dbgraph_graph::impact::{ImpactAnalyzer, ImpactOptions, ImpactReport};
use dbgraph_graph::relations::{relations_for, Direction, RelationsOptions, RelationsReport};
use dbgraph_graph::search::{search_snapshot, SearchOptions, SearchResult};
use dbgraph_sql::{analyze_sql, SqlDialect, SqlParser};
use dbgraph_storage::GraphRepository;
use serde::Serialize;

use crate::commands::common::{discover_context, latest_snapshot, require_graph_index};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ValidateSqlReport {
    pub(crate) valid: bool,
    pub(crate) dialect: String,
    pub(crate) normalized_sql: String,
    pub(crate) diagnostics: Vec<String>,
    pub(crate) unresolved: Vec<UnresolvedSqlReference>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UnresolvedSqlReference {
    pub(crate) kind: String,
    pub(crate) name: String,
    pub(crate) suggestions: Vec<String>,
}

pub(crate) fn read_sql_input(sql: Option<String>, file: Option<&Path>) -> Result<String> {
    match (sql, file) {
        (Some(sql), None) => Ok(sql),
        (None, Some(file)) => {
            fs::read_to_string(file).map_err(|source| DbGraphError::io(file, source))
        }
        (Some(_), Some(_)) => Err(DbGraphError::invalid_argument(
            "`validate-sql` accepts only one of `--sql` or `--file`",
        )),
        (None, None) => Err(DbGraphError::invalid_argument(
            "`validate-sql` requires `--sql <SQL>` or `--file <PATH>`",
        )),
    }
}

pub(crate) fn validate_sql(
    start: impl AsRef<Path>,
    sql: &str,
    dialect: SqlDialect,
) -> Result<ValidateSqlReport> {
    let parsed = SqlParser::new(dialect).parse(sql)?;
    let analysis = analyze_sql(sql, dialect)?;
    let context = ProjectContext::discover_from(start.as_ref())?
        .unwrap_or_else(|| ProjectContext::from_project_root(start.as_ref()));
    let latest = SnapshotStore::new(&context).read_latest()?;
    let mut unresolved = Vec::new();
    if let Some(snapshot) = latest {
        let repository = GraphRepository::open(context.graph_db_path())?;
        for reference in analysis.references {
            if !reference_exists(&snapshot, &reference.object_name) {
                unresolved.push(UnresolvedSqlReference {
                    kind: reference.kind.as_str().to_owned(),
                    name: reference.object_name.clone(),
                    suggestions: suggest_objects(&snapshot, &repository, &reference.object_name)?,
                });
            }
        }
    }

    Ok(ValidateSqlReport {
        valid: parsed.status == dbgraph_sql::ParseStatus::Parsed,
        dialect: dialect.as_str().to_owned(),
        normalized_sql: parsed.normalized_sql,
        diagnostics: parsed
            .diagnostics
            .into_iter()
            .chain(analysis.diagnostics)
            .map(|diagnostic| diagnostic.message)
            .collect(),
        unresolved,
    })
}

fn reference_exists(snapshot: &dbgraph_core::model::DbSnapshot, name: &str) -> bool {
    let normalized = normalize_sql_name(name);
    snapshot.objects.iter().any(|object| {
        matches!(
            object.kind,
            dbgraph_core::model::DbObjectKind::Table
                | dbgraph_core::model::DbObjectKind::View
                | dbgraph_core::model::DbObjectKind::MaterializedView
                | dbgraph_core::model::DbObjectKind::Column
        ) && (normalize_sql_name(&object.full_name) == normalized
            || normalize_sql_name(&object.name) == normalized)
    })
}

fn suggest_objects(
    snapshot: &dbgraph_core::model::DbSnapshot,
    repository: &GraphRepository,
    name: &str,
) -> Result<Vec<String>> {
    let normalized = normalize_sql_name(name);
    let singular = normalized.trim_end_matches('s').to_owned();
    let plural = format!("{singular}s");
    let (table_hint, column_hint) = table_column_hint(name);
    let mut suggestions = snapshot
        .objects
        .iter()
        .filter(|object| {
            matches!(
                object.kind,
                dbgraph_core::model::DbObjectKind::Table
                    | dbgraph_core::model::DbObjectKind::View
                    | dbgraph_core::model::DbObjectKind::MaterializedView
                    | dbgraph_core::model::DbObjectKind::Column
            )
        })
        .filter(|object| {
            let object_name = normalize_sql_name(&object.name);
            if object.kind == dbgraph_core::model::DbObjectKind::Column {
                let table_matches = table_hint.as_ref().map_or(true, |hint| {
                    object
                        .table_name
                        .as_deref()
                        .is_some_and(|table| normalize_sql_name(table) == *hint)
                        || object.full_name.to_ascii_lowercase().contains(hint)
                });
                let column = column_hint.as_deref().unwrap_or(&normalized);
                table_matches
                    && (object_name == column
                        || object_name.contains(column)
                        || edit_distance(&object_name, column) <= 2)
            } else {
                object_name == singular || object_name == plural || object_name.contains(&singular)
            }
        })
        .map(|object| object.full_name.clone())
        .collect::<Vec<_>>();
    for object in repository.search_objects(&normalized)? {
        if !suggestions.contains(&object.full_name) {
            suggestions.push(object.full_name);
        }
    }
    suggestions.sort();
    suggestions.dedup();
    suggestions.truncate(5);
    Ok(suggestions)
}

fn table_column_hint(name: &str) -> (Option<String>, Option<String>) {
    let parts = name
        .split('.')
        .map(normalize_sql_name)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    match parts.as_slice() {
        [table, column] | [_, table, column] => (Some(table.clone()), Some(column.clone())),
        [column] => (None, Some(column.clone())),
        _ => (None, None),
    }
}

fn edit_distance(left: &str, right: &str) -> usize {
    let mut costs = (0..=right.len()).collect::<Vec<_>>();
    for (left_idx, left_char) in left.chars().enumerate() {
        let mut previous = costs[0];
        costs[0] = left_idx + 1;
        for (right_idx, right_char) in right.chars().enumerate() {
            let insert = costs[right_idx + 1] + 1;
            let delete = costs[right_idx] + 1;
            let replace = previous + usize::from(left_char != right_char);
            previous = costs[right_idx + 1];
            costs[right_idx + 1] = insert.min(delete).min(replace);
        }
    }
    *costs.last().unwrap_or(&0)
}

fn normalize_sql_name(value: &str) -> String {
    value
        .rsplit('.')
        .next()
        .unwrap_or(value)
        .trim_matches('"')
        .trim_matches('`')
        .to_ascii_lowercase()
}

pub(crate) fn print_validate_sql_report(report: &ValidateSqlReport) {
    println!("SQL validation");
    println!("Dialect: {}", report.dialect);
    println!("Parse: {}", if report.valid { "valid" } else { "invalid" });
    if !report.diagnostics.is_empty() {
        println!("Diagnostics:");
        for diagnostic in &report.diagnostics {
            println!("  - {diagnostic}");
        }
    }
    if !report.unresolved.is_empty() {
        println!("Unresolved references:");
        for item in &report.unresolved {
            if item.suggestions.is_empty() {
                println!("  - {} {}", item.kind, item.name);
            } else {
                println!(
                    "  - {} {} (suggestions: {})",
                    item.kind,
                    item.name,
                    item.suggestions.join(", ")
                );
            }
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SearchReport {
    pub(crate) query: String,
    pub(crate) results: Vec<SearchResult>,
}

pub(crate) fn search_project(
    start: impl AsRef<Path>,
    query: &str,
    kind: Option<&str>,
) -> Result<SearchReport> {
    let context = discover_context(start.as_ref())?;
    require_graph_index(&context)?;
    let snapshot = latest_snapshot(&context)?;
    let query = kind.map_or_else(|| query.to_owned(), |kind| format!("kind:{kind} {query}"));
    Ok(SearchReport {
        results: search_snapshot(&snapshot, &query, &SearchOptions::default()),
        query,
    })
}

pub(crate) fn print_search_report(report: &SearchReport) {
    println!("DbGraph search: {}", report.query);
    for result in &report.results {
        println!(
            "- {} {} :: {}",
            result.kind, result.full_name, result.summary
        );
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TableReport {
    pub(crate) table: String,
    pub(crate) columns: Vec<ColumnReport>,
    pub(crate) constraints: Vec<ObjectSummary>,
    pub(crate) indexes: Vec<ObjectSummary>,
    pub(crate) profile: Option<TableProfile>,
    pub(crate) incoming_relations: Vec<EdgeSummary>,
    pub(crate) outgoing_relations: Vec<EdgeSummary>,
    pub(crate) suggestions: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ColumnReport {
    pub(crate) name: String,
    pub(crate) full_name: String,
    pub(crate) data_type: Option<String>,
    pub(crate) nullable: Option<bool>,
    pub(crate) default: Option<String>,
    pub(crate) profile: Option<ColumnProfile>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ObjectSummary {
    pub(crate) kind: String,
    pub(crate) full_name: String,
    pub(crate) summary: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EdgeSummary {
    pub(crate) kind: String,
    pub(crate) from: String,
    pub(crate) to: String,
    pub(crate) confidence: f64,
    pub(crate) evidence: Vec<String>,
}

pub(crate) fn table_project(start: impl AsRef<Path>, table_name: &str) -> Result<TableReport> {
    let context = discover_context(start.as_ref())?;
    require_graph_index(&context)?;
    let snapshot = latest_snapshot(&context)?;
    let Some(table) = resolve_table(&snapshot, table_name) else {
        return Ok(TableReport {
            table: table_name.to_owned(),
            columns: Vec::new(),
            constraints: Vec::new(),
            indexes: Vec::new(),
            profile: None,
            incoming_relations: Vec::new(),
            outgoing_relations: Vec::new(),
            suggestions: table_suggestions(&snapshot, table_name),
        });
    };
    let columns = snapshot
        .objects
        .iter()
        .filter(|object| {
            object.kind == DbObjectKind::Column
                && object.table_name.as_deref()
                    == table.table_name.as_deref().or(Some(table.name.as_str()))
        })
        .map(|object| ColumnReport {
            name: object
                .column_name
                .clone()
                .unwrap_or_else(|| object.name.clone()),
            full_name: object.full_name.clone(),
            data_type: object
                .column
                .as_ref()
                .and_then(|column| column.data_type.clone()),
            nullable: object.column.as_ref().and_then(|column| column.nullable),
            default: object
                .column
                .as_ref()
                .and_then(|column| column.default.clone()),
            profile: snapshot
                .column_profiles
                .iter()
                .find(|profile| profile.object_id == object.id)
                .cloned(),
        })
        .collect::<Vec<_>>();
    let constraints = snapshot
        .objects
        .iter()
        .filter(|object| {
            matches!(
                object.kind,
                DbObjectKind::PrimaryKey
                    | DbObjectKind::ForeignKey
                    | DbObjectKind::UniqueConstraint
                    | DbObjectKind::CheckConstraint
            ) && object.table_name == table.table_name
        })
        .map(object_summary)
        .collect();
    let indexes = snapshot
        .objects
        .iter()
        .filter(|object| {
            object.kind == DbObjectKind::Index && object.table_name == table.table_name
        })
        .map(object_summary)
        .collect();
    Ok(TableReport {
        table: table.full_name.clone(),
        columns,
        constraints,
        indexes,
        profile: snapshot
            .table_profiles
            .iter()
            .find(|profile| profile.object_id == table.id)
            .cloned(),
        incoming_relations: snapshot
            .edges
            .iter()
            .filter(|edge| edge.to_object_id == table.id)
            .map(|edge| edge_summary(&snapshot, edge))
            .collect(),
        outgoing_relations: snapshot
            .edges
            .iter()
            .filter(|edge| edge.from_object_id == table.id)
            .map(|edge| edge_summary(&snapshot, edge))
            .collect(),
        suggestions: Vec::new(),
    })
}

pub(crate) fn print_table_report(report: &TableReport) {
    println!("Table: {}", report.table);
    if !report.suggestions.is_empty() {
        println!("Not found. Suggestions: {}", report.suggestions.join(", "));
        return;
    }
    println!("Columns:");
    for column in &report.columns {
        println!(
            "- {} {:?} nullable={:?} default={:?}",
            column.name, column.data_type, column.nullable, column.default
        );
    }
    println!("Constraints: {}", report.constraints.len());
    println!("Indexes: {}", report.indexes.len());
    println!("Incoming relations: {}", report.incoming_relations.len());
    println!("Outgoing relations: {}", report.outgoing_relations.len());
}

pub(crate) fn relations_project(
    start: impl AsRef<Path>,
    object: &str,
    depth: usize,
    direction: Direction,
) -> Result<RelationsReport> {
    let context = discover_context(start.as_ref())?;
    require_graph_index(&context)?;
    let snapshot = latest_snapshot(&context)?;
    relations_for(&snapshot, object, &RelationsOptions { depth, direction })
}

pub(crate) fn print_relations_report(report: &RelationsReport) {
    println!("Relations for {}", report.target);
    for path in &report.paths {
        println!("- {}", path.objects.join(" -> "));
        for edge in &path.edges {
            println!(
                "  {} {} -> {} confidence={}",
                edge.kind, edge.from, edge.to, edge.confidence
            );
        }
    }
}

pub(crate) fn context_project(
    start: impl AsRef<Path>,
    query: &str,
    token_budget: usize,
) -> Result<ContextPackage> {
    let context = discover_context(start.as_ref())?;
    require_graph_index(&context)?;
    let snapshot = latest_snapshot(&context)?;
    Ok(ContextBuilder::new(RankingWeights::default()).build(
        &snapshot,
        query,
        &ContextOptions {
            token_budget,
            max_objects: 12,
        },
    ))
}

pub(crate) fn print_context_report(report: &ContextPackage) {
    println!("## DbGraph Context");
    println!();
    println!("Query: {}", report.query);
    println!();
    println!("Relevant objects:");
    for object in &report.objects {
        println!(
            "- {} {} :: {}",
            object.kind, object.full_name, object.summary
        );
    }
    if !report.relation_paths.is_empty() {
        println!();
        println!("Relation paths:");
        for path in &report.relation_paths {
            println!("- {path}");
        }
    }
    println!();
    println!("Risks:");
    for risk in &report.risks {
        println!("- {risk}");
    }
    println!();
    println!("Suggested next tools:");
    for tool in &report.suggested_next_tools {
        println!("- {tool}");
    }
}

pub(crate) fn diff_project(start: impl AsRef<Path>) -> Result<SchemaDiff> {
    let context = discover_context(start.as_ref())?;
    let store = SnapshotStore::new(&context);
    let latest = store.read_latest()?.ok_or_else(|| {
        DbGraphError::invalid_config("no snapshots found; run `dbgraph snapshot` first")
    })?;
    let previous_path = store.previous_snapshot_path()?.ok_or_else(|| {
        DbGraphError::invalid_config(
            "no previous snapshot found; run `dbgraph snapshot` at least twice",
        )
    })?;
    let previous = store.read_snapshot(previous_path)?;
    Ok(DiffEngine::compare(&previous, &latest))
}

pub(crate) fn print_diff_report(report: &SchemaDiff) {
    println!(
        "Schema diff: {} -> {}",
        report.previous_snapshot_id, report.latest_snapshot_id
    );
    println!("Schema hash changed: {}", report.schema_hash_changed);
    for change in &report.changes {
        println!(
            "- {:?} {} {}",
            change.kind,
            change.object_kind.as_str(),
            change.full_name
        );
    }
    for candidate in &report.rename_candidates {
        println!(
            "- rename candidate: {} -> {} ({})",
            candidate.from_full_name, candidate.to_full_name, candidate.reason
        );
    }
}

pub(crate) fn impact_project(
    start: impl AsRef<Path>,
    object: &str,
    depth: usize,
) -> Result<ImpactReport> {
    let context = discover_context(start.as_ref())?;
    require_graph_index(&context)?;
    let snapshot = latest_snapshot(&context)?;
    match ImpactAnalyzer::new().analyze(&snapshot, object, &ImpactOptions { depth }) {
        Ok(report) => Ok(report),
        Err(err) => {
            let suggestions = table_suggestions(&snapshot, object);
            if suggestions.is_empty() {
                Err(err)
            } else {
                Err(DbGraphError::invalid_argument(format!(
                    "{err}. Suggestions: {}",
                    suggestions.join(", ")
                )))
            }
        }
    }
}

pub(crate) fn print_impact_report(report: &ImpactReport) {
    println!("Impact for {}", report.target);
    for item in &report.items {
        println!(
            "- {:?} {} {} ({})",
            item.scope, item.kind, item.full_name, item.evidence
        );
    }
    if !report.risks.is_empty() {
        println!("Risks:");
        for risk in &report.risks {
            println!("- {} ({})", risk.message, risk.evidence);
        }
    }
}

fn resolve_table<'a>(snapshot: &'a DbSnapshot, table_name: &str) -> Option<&'a DbObject> {
    let normalized = table_name.to_ascii_lowercase();
    snapshot.objects.iter().find(|object| {
        object.kind == DbObjectKind::Table
            && (object.full_name.eq_ignore_ascii_case(table_name)
                || object.name.eq_ignore_ascii_case(table_name)
                || object
                    .full_name
                    .to_ascii_lowercase()
                    .ends_with(&format!(".{normalized}")))
    })
}

fn table_suggestions(snapshot: &DbSnapshot, table_name: &str) -> Vec<String> {
    let normalized = normalize_sql_name(table_name);
    let mut suggestions = snapshot
        .objects
        .iter()
        .filter(|object| object.kind == DbObjectKind::Table)
        .filter(|object| {
            let name = normalize_sql_name(&object.name);
            name.contains(&normalized) || edit_distance(&name, &normalized) <= 2
        })
        .map(|object| object.full_name.clone())
        .collect::<Vec<_>>();
    suggestions.sort();
    suggestions.truncate(5);
    suggestions
}

fn object_summary(object: &DbObject) -> ObjectSummary {
    ObjectSummary {
        kind: object.kind.as_str().to_owned(),
        full_name: object.full_name.clone(),
        summary: object
            .metadata
            .get("comment")
            .and_then(|value| value.as_str())
            .map_or_else(
                || format!("{} {}", object.kind.as_str(), object.full_name),
                ToOwned::to_owned,
            ),
    }
}

fn edge_summary(snapshot: &DbSnapshot, edge: &DbEdge) -> EdgeSummary {
    let object_name = |id: &str| {
        snapshot
            .objects
            .iter()
            .find(|object| object.id == id)
            .map_or_else(|| id.to_owned(), |object| object.full_name.clone())
    };
    EdgeSummary {
        kind: edge.kind.as_str().to_owned(),
        from: object_name(&edge.from_object_id),
        to: object_name(&edge.to_object_id),
        confidence: edge.confidence,
        evidence: edge
            .evidence
            .iter()
            .map(|evidence| evidence.detail.clone())
            .collect(),
    }
}
