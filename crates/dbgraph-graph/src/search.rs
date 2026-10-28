//! Search query parsing and snapshot search.

use std::collections::BTreeSet;
use std::path::Path;

use dbgraph_core::model::{DbObject, DbSnapshot};
use serde::{Deserialize, Serialize};

use crate::evidence::{object_suggestions, SnapshotProvenance, ToolSuggestion};

/// Parsed structured search query.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ParsedSearchQuery {
    /// Free text terms.
    pub text: String,
    /// Kind filters.
    pub kinds: Vec<String>,
    /// Schema filters.
    pub schema_filters: Vec<String>,
    /// Name filters.
    pub name_filters: Vec<String>,
}

/// Search options.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchOptions {
    /// Maximum number of results.
    pub limit: usize,
}

impl Default for SearchOptions {
    fn default() -> Self {
        Self { limit: 20 }
    }
}

/// The filters that constrained the lexical search.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchFilters {
    /// Allowed object kinds (OR within this field).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub kinds: Vec<String>,
    /// Allowed schemas (OR within this field).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub schema_filters: Vec<String>,
    /// Name substrings (OR within this field).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub name_filters: Vec<String>,
}

/// Denominators for the snapshot search, before any context graph expansion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchCoverage {
    /// All objects captured in the snapshot, not a live catalog count.
    pub snapshot_objects: usize,
    /// Objects passing all filter fields.
    pub candidate_objects: usize,
    /// Candidates matching at least one term, or the filter-only query.
    pub matched_objects: usize,
    /// Matches returned before later context-budget selection.
    pub returned_objects: usize,
    /// Matches excluded by the search result limit.
    pub omitted_by_limit: usize,
}

/// Evidence about this lexical query, not proof of business-concept absence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchDiagnostics {
    /// `matched`, `empty_snapshot`, `no_search_terms`, `no_candidates`, or `no_term_match`.
    pub status: String,
    /// Name/full-name/metadata lexical matching of any extracted term.
    pub match_mode: String,
    /// Terms observed in at least one filtered candidate (OR search).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub matched_terms: Vec<String>,
    /// Terms checked against filtered candidates without a lexical match.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unmatched_terms: Vec<String>,
    /// Terms not checked because the snapshot or candidate set was empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unchecked_terms: Vec<String>,
    /// Filters applied to candidate objects.
    pub filters: SearchFilters,
    /// Search denominators and result-limit omissions.
    pub coverage: SearchCoverage,
}

/// Shared CLI/MCP search contract.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchReport {
    /// Original query, including any explicit filters.
    pub query: String,
    /// Local snapshot identity and capture time.
    pub snapshot: SnapshotProvenance,
    /// Lexical evidence and coverage.
    pub diagnostics: SearchDiagnostics,
    /// Ranked matches, never guesses for an unmatched query.
    pub results: Vec<SearchResult>,
    /// Concrete calls based on returned snapshot objects.
    pub suggested_next_tools: Vec<ToolSuggestion>,
}

/// One search result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    /// Object id.
    pub id: String,
    /// Object kind.
    pub kind: String,
    /// Object short name.
    pub name: String,
    /// Fully qualified name.
    pub full_name: String,
    /// Summary text.
    pub summary: String,
    /// Deterministic score.
    pub score: f64,
    /// Terms matching this object's name, full name, or metadata.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub matched_terms: Vec<String>,
    /// Existing SQL source path, when recorded in object metadata.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_path: Option<String>,
}

/// Parses field-qualified search syntax.
#[must_use]
pub fn parse_search_query(raw: &str) -> ParsedSearchQuery {
    let mut parsed = ParsedSearchQuery::default();
    let mut text = Vec::new();
    for token in tokenize(raw) {
        let Some((key, value)) = token.split_once(':') else {
            text.push(token);
            continue;
        };
        let value = unquote(value);
        if value.is_empty() {
            text.push(token);
            continue;
        }
        match key.to_ascii_lowercase().as_str() {
            "kind" => parsed.kinds.push(value.to_ascii_lowercase()),
            "schema" => parsed.schema_filters.push(value.to_lowercase()),
            "name" => parsed.name_filters.push(value),
            _ => text.push(token),
        }
    }
    text.join(" ").trim().clone_into(&mut parsed.text);
    parsed
}

/// Searches snapshot objects with deterministic ranking.
#[must_use]
pub fn search_snapshot(
    snapshot: &DbSnapshot,
    query: &str,
    options: &SearchOptions,
    project_path: &Path,
) -> SearchReport {
    let parsed = parse_search_query(query);
    let terms = extract_search_terms(&parsed.text);
    // A filter-only query deliberately browses that subset. Free text that has
    // no searchable terms must not silently turn into a whole-snapshot query.
    let no_search_terms = terms.is_empty()
        && (!parsed.text.trim().is_empty()
            || (parsed.kinds.is_empty()
                && parsed.schema_filters.is_empty()
                && parsed.name_filters.is_empty()));
    let mut candidate_objects = 0;
    let mut observed_terms = BTreeSet::new();
    let mut matches = Vec::new();
    for object in &snapshot.objects {
        if !matches_filters(object, &parsed) {
            continue;
        }
        candidate_objects += 1;
        if no_search_terms {
            continue;
        }
        let haystack = if terms.is_empty() {
            String::new()
        } else {
            object_haystack(object)
        };
        let (score, matched_terms) = score_object(object, &haystack, &terms, &parsed);
        observed_terms.extend(matched_terms.iter().cloned());
        if score > 0.0 || terms.is_empty() {
            matches.push((
                SearchResult {
                    id: object.id.clone(),
                    kind: object.kind.as_str().to_owned(),
                    name: object.name.clone(),
                    full_name: object.full_name.clone(),
                    summary: object_summary(object),
                    score,
                    matched_terms,
                    source_path: object
                        .metadata
                        .get("sourcePath")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_owned),
                },
                object,
            ));
        }
    }
    matches.sort_by(|(left, _), (right, _)| {
        right
            .score
            .partial_cmp(&left.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.kind.cmp(&right.kind))
            .then_with(|| left.full_name.cmp(&right.full_name))
    });
    let matched_objects = matches.len();
    matches.truncate(options.limit);
    let suggested_next_tools = object_suggestions(
        snapshot,
        matches.iter().map(|(_, object)| *object),
        project_path,
    );
    let results = matches
        .into_iter()
        .map(|(result, _)| result)
        .collect::<Vec<_>>();
    let diagnostics = search_diagnostics(
        parsed,
        terms,
        observed_terms,
        SearchCoverage {
            snapshot_objects: snapshot.objects.len(),
            candidate_objects,
            matched_objects,
            returned_objects: results.len(),
            omitted_by_limit: matched_objects.saturating_sub(results.len()),
        },
        no_search_terms,
    );
    SearchReport {
        query: query.to_owned(),
        snapshot: SnapshotProvenance::from_snapshot(snapshot),
        diagnostics,
        results,
        suggested_next_tools,
    }
}

fn search_diagnostics(
    parsed: ParsedSearchQuery,
    terms: Vec<String>,
    observed_terms: BTreeSet<String>,
    coverage: SearchCoverage,
    no_search_terms: bool,
) -> SearchDiagnostics {
    let mut unmatched_terms = Vec::new();
    let mut unchecked_terms = Vec::new();
    for term in terms {
        if coverage.candidate_objects == 0 {
            unchecked_terms.push(term);
        } else if !observed_terms.contains(&term) {
            unmatched_terms.push(term);
        }
    }
    let status = if coverage.snapshot_objects == 0 {
        "empty_snapshot"
    } else if no_search_terms {
        "no_search_terms"
    } else if coverage.candidate_objects == 0 {
        "no_candidates"
    } else if coverage.matched_objects == 0 {
        "no_term_match"
    } else {
        "matched"
    };
    SearchDiagnostics {
        status: status.to_owned(),
        match_mode: "lexical_or".to_owned(),
        matched_terms: observed_terms.into_iter().collect(),
        unmatched_terms,
        unchecked_terms,
        filters: SearchFilters {
            kinds: parsed.kinds,
            schema_filters: parsed.schema_filters,
            name_filters: parsed.name_filters,
        },
        coverage,
    }
}

/// Extracts useful terms from natural-language text.
#[must_use]
pub fn extract_search_terms(query: &str) -> Vec<String> {
    let mut terms = Vec::new();
    for word in query.split(|ch: char| !ch.is_alphanumeric()) {
        let lower = word.to_lowercase();
        if lower.is_empty() || is_stop_word(&lower) {
            continue;
        }
        terms.push(lower.clone());
        if let Some(stripped) = lower.strip_suffix('s') {
            if stripped.len() >= 3 {
                terms.push(stripped.to_owned());
            }
        }
    }
    terms.sort();
    terms.dedup();
    terms
}

fn tokenize(raw: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_quote = false;
    for ch in raw.chars() {
        match ch {
            '"' => {
                in_quote = !in_quote;
                current.push(ch);
            }
            ch if ch.is_whitespace() && !in_quote => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(ch),
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

fn unquote(value: &str) -> String {
    value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .unwrap_or(value)
        .to_owned()
}

fn matches_filters(object: &DbObject, parsed: &ParsedSearchQuery) -> bool {
    (parsed.kinds.is_empty()
        || parsed
            .kinds
            .iter()
            .any(|kind| object.kind.as_str() == kind.as_str()))
        && (parsed.schema_filters.is_empty()
            || object.schema_name.as_ref().is_some_and(|schema| {
                let schema = schema.to_lowercase();
                parsed.schema_filters.contains(&schema)
            }))
        && (parsed.name_filters.is_empty()
            || parsed.name_filters.iter().any(|filter| {
                let filter = filter.to_lowercase();
                object.name.to_lowercase().contains(&filter)
                    || object.full_name.to_lowercase().contains(&filter)
            }))
}

fn score_object(
    object: &DbObject,
    haystack: &str,
    terms: &[String],
    parsed: &ParsedSearchQuery,
) -> (f64, Vec<String>) {
    let mut score = 0.0;
    let mut matched_terms = Vec::new();
    let name = object.name.to_lowercase();
    let full_name = object.full_name.to_lowercase();
    for term in terms {
        let mut matched = false;
        if full_name == *term || name == *term {
            score += 50.0;
            matched = true;
        } else if name.starts_with(term) {
            score += 25.0;
            matched = true;
        } else if name.contains(term) {
            score += 15.0;
            matched = true;
        }
        if haystack.contains(term) {
            score += 10.0;
            matched = true;
        }
        if matched {
            matched_terms.push(term.clone());
        }
    }
    if matched_terms.is_empty() && !terms.is_empty() {
        return (0.0, matched_terms);
    }
    score += kind_bonus(object.kind.as_str());
    if !parsed.kinds.is_empty() {
        score += 5.0;
    }
    (score, matched_terms)
}

fn kind_bonus(kind: &str) -> f64 {
    match kind {
        "table" | "view" | "materialized_view" => 12.0,
        "column" => 8.0,
        "query" | "sql_artifact" => 6.0,
        "foreign_key" | "primary_key" | "unique_constraint" | "check_constraint" => 5.0,
        _ => 1.0,
    }
}

fn object_haystack(object: &DbObject) -> String {
    format!(
        "{} {} {}",
        object.name,
        object.full_name,
        serde_json::to_string(&object.metadata).unwrap_or_default()
    )
    .to_lowercase()
}

fn object_summary(object: &DbObject) -> String {
    if let Some(comment) = object
        .metadata
        .get("comment")
        .and_then(|value| value.as_str())
    {
        return comment.to_owned();
    }
    if let Some(comment) = object
        .table
        .as_ref()
        .and_then(|table| table.comment.clone())
    {
        return comment;
    }
    if let Some(comment) = object
        .column
        .as_ref()
        .and_then(|column| column.comment.clone())
    {
        return comment;
    }
    object
        .metadata
        .get("normalizedSql")
        .and_then(|value| value.as_str())
        .map_or_else(
            || format!("{} {}", object.kind.as_str(), object.full_name),
            |sql| sql.chars().take(160).collect(),
        )
}

fn is_stop_word(value: &str) -> bool {
    matches!(
        value,
        "the" | "and" | "for" | "with" | "from" | "this" | "that" | "show" | "give" | "about"
    )
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::search::{parse_search_query, search_snapshot, SearchOptions};
    use dbgraph_core::model::{DbObject, DbObjectKind, DbSnapshot};

    #[test]
    fn parses_structured_filters_and_free_text() {
        let parsed = parse_search_query(r#"kind:table name:"payment events" schema:public refund"#);

        assert_eq!(parsed.text, "refund");
        assert_eq!(parsed.kinds, vec!["table"]);
        assert_eq!(parsed.name_filters, vec!["payment events"]);
        assert_eq!(parsed.schema_filters, vec!["public"]);
    }

    #[test]
    fn search_finds_payment_comments_filters_kind_and_sorts_stably() {
        let snapshot = sample_snapshot();

        let report = search_snapshot(
            &snapshot,
            "kind:table payment",
            &SearchOptions::default(),
            Path::new("."),
        );
        let results = report.results;

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].kind, "table");
        assert_eq!(results[0].full_name, "public.payments");
        assert!(results[0].summary.contains("payment"));
    }

    #[test]
    fn search_preserves_unicode_terms_and_short_identifiers() {
        let mut snapshot = sample_snapshot();
        snapshot.objects[0].metadata.insert(
            "semantic".to_owned(),
            serde_json::json!({"description": "退款记录"}),
        );
        snapshot.objects.push(DbObject::new(
            "column:orders.id",
            DbObjectKind::Column,
            "public.orders.id",
        ));

        let chinese = search_snapshot(
            &snapshot,
            "kind:table 退款",
            &SearchOptions::default(),
            Path::new("."),
        )
        .results;
        assert_eq!(chinese.len(), 1);
        assert_eq!(chinese[0].full_name, "public.orders");
        let short =
            search_snapshot(&snapshot, "id", &SearchOptions::default(), Path::new(".")).results;
        assert_eq!(short.len(), 1);
        assert_eq!(short[0].full_name, "public.orders.id");
    }

    #[test]
    fn search_returns_no_matches_without_terms_but_allows_filter_only_queries() {
        let snapshot = sample_snapshot();
        for query in ["不存在", "unmatched", "!!!", "the and", ""] {
            assert_eq!(
                search_snapshot(&snapshot, query, &SearchOptions::default(), Path::new("."))
                    .results,
                [] as [super::SearchResult; 0]
            );
        }
        let filtered = search_snapshot(
            &snapshot,
            "kind:table schema:public name:order",
            &SearchOptions::default(),
            Path::new("."),
        )
        .results;
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].full_name, "public.orders");
    }

    #[test]
    fn empty_results_distinguish_snapshot_terms_candidates_and_lexical_misses() {
        let snapshot = sample_snapshot();
        let options = SearchOptions::default();
        let project = Path::new(".");
        let empty = search_snapshot(
            &DbSnapshot::new("empty", "sqlite", "empty", 9),
            "orders",
            &options,
            project,
        );
        assert_eq!(empty.diagnostics.status, "empty_snapshot");
        assert_eq!(empty.diagnostics.coverage.snapshot_objects, 0);
        assert!(empty.diagnostics.unmatched_terms.is_empty());
        assert!(empty
            .diagnostics
            .unchecked_terms
            .contains(&"orders".to_owned()));

        let no_terms = search_snapshot(&snapshot, "the and !!!", &options, project);
        assert_eq!(no_terms.diagnostics.status, "no_search_terms");
        assert_eq!(
            no_terms.diagnostics.coverage.candidate_objects,
            snapshot.objects.len()
        );

        let filtered = search_snapshot(&snapshot, "kind:view orders", &options, project);
        assert_eq!(filtered.diagnostics.status, "no_candidates");
        assert_eq!(filtered.diagnostics.coverage.candidate_objects, 0);
        assert!(filtered.diagnostics.unmatched_terms.is_empty());
        assert!(!filtered.diagnostics.unchecked_terms.is_empty());

        let unmatched = search_snapshot(&snapshot, "kind:table unknownconcept", &options, project);
        assert_eq!(unmatched.diagnostics.status, "no_term_match");
        assert_eq!(unmatched.diagnostics.coverage.candidate_objects, 2);
        assert_eq!(
            unmatched.diagnostics.unmatched_terms,
            vec!["unknownconcept"]
        );
        assert!(unmatched.diagnostics.unchecked_terms.is_empty());
        for report in [empty, no_terms, filtered, unmatched] {
            assert!(report.results.is_empty());
            assert!(report.suggested_next_tools.is_empty());
        }
    }

    #[test]
    fn partial_or_matches_and_limits_keep_full_candidate_diagnostics() {
        let mut snapshot = sample_snapshot();
        snapshot.objects[0].metadata.insert(
            "semantic".to_owned(),
            serde_json::json!({"description": "退款"}),
        );
        let report = search_snapshot(
            &snapshot,
            "kind:table payment 退款 unknownconcept",
            &SearchOptions { limit: 1 },
            Path::new("another-project"),
        );

        assert_eq!(report.diagnostics.status, "matched");
        assert_eq!(report.diagnostics.coverage.matched_objects, 2);
        assert_eq!(report.diagnostics.coverage.returned_objects, 1);
        assert_eq!(report.diagnostics.coverage.omitted_by_limit, 1);
        assert_eq!(report.diagnostics.matched_terms, vec!["payment", "退款"]);
        assert_eq!(report.diagnostics.unmatched_terms, vec!["unknownconcept"]);
        assert!(!report.results[0].matched_terms.is_empty());
        assert_eq!(report.suggested_next_tools.len(), 1);
        assert_eq!(
            report.suggested_next_tools[0].arguments["projectPath"],
            "another-project"
        );
        assert_eq!(report.snapshot.id, snapshot.id);
        assert_eq!(
            report.snapshot.created_at_unix_ms,
            snapshot.created_at_unix_ms
        );
    }

    #[test]
    fn filter_only_browsing_reports_coverage_and_sql_sources_survive() {
        let mut snapshot = sample_snapshot();
        let mut query = DbObject::new(
            "query:sql/orders.sql",
            dbgraph_core::model::DbObjectKind::Query,
            "sql.orders.sql",
        );
        query
            .metadata
            .insert("sourcePath".to_owned(), serde_json::json!("sql/orders.sql"));
        query.metadata.insert(
            "normalizedSql".to_owned(),
            serde_json::json!("SELECT id FROM public.orders"),
        );
        snapshot.objects.push(query);
        let report = search_snapshot(
            &snapshot,
            "kind:query",
            &SearchOptions::default(),
            Path::new("."),
        );

        assert_eq!(report.diagnostics.status, "matched");
        assert_eq!(report.diagnostics.coverage.candidate_objects, 1);
        assert!(report.diagnostics.matched_terms.is_empty());
        assert_eq!(
            report.results[0].source_path.as_deref(),
            Some("sql/orders.sql")
        );
        assert_eq!(report.suggested_next_tools[0].tool, "dbgraph_validate_sql");
        let json = serde_json::to_string(&report).expect("search report JSON");
        let restored: super::SearchReport =
            serde_json::from_str(&json).expect("omitted empty term lists deserialize");
        assert_eq!(restored, report);
    }

    fn sample_snapshot() -> DbSnapshot {
        let mut snapshot = DbSnapshot::new("s1", "postgres", "app", 1);
        let mut payments = DbObject::new("table:payments", DbObjectKind::Table, "public.payments");
        payments.schema_name = Some("public".to_owned());
        payments.metadata.insert(
            "comment".to_owned(),
            serde_json::Value::String("payment ledger records".to_owned()),
        );
        let mut orders = DbObject::new("table:orders", DbObjectKind::Table, "public.orders");
        orders.schema_name = Some("public".to_owned());
        let mut status = DbObject::new(
            "column:payments.status",
            DbObjectKind::Column,
            "public.payments.status",
        );
        status.schema_name = Some("public".to_owned());
        status.table_name = Some("payments".to_owned());
        snapshot.objects = vec![orders, status, payments];
        snapshot
    }
}
