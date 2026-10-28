//! Context candidate retrieval and formatting.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

use dbgraph_core::model::{DbEdge, DbObject, DbSnapshot};
use dbgraph_core::semantics::{semantic_description, semantic_metadata};
use dbgraph_core::{DbGraphError, Result};
use serde::{Deserialize, Serialize};

use crate::evidence::{SnapshotProvenance, ToolSuggestion};
use crate::search::{search_snapshot, SearchDiagnostics, SearchOptions};
use crate::workload::WorkloadModel;

/// Configurable ranking weights.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RankingWeights {
    /// Name/text match weight.
    pub text_match: f64,
    /// Graph-neighbor weight.
    pub graph_neighbor: f64,
    /// Table/profile weight.
    pub statistics: f64,
    /// SQL workload frequency weight.
    pub workload: f64,
}

impl Default for RankingWeights {
    fn default() -> Self {
        Self {
            text_match: 1.0,
            graph_neighbor: 0.35,
            statistics: 0.1,
            workload: 0.2,
        }
    }
}

/// Context builder options.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContextOptions {
    /// Rough token budget.
    pub token_budget: usize,
    /// Maximum objects.
    pub max_objects: usize,
}

impl Default for ContextOptions {
    fn default() -> Self {
        Self {
            token_budget: 800,
            max_objects: 12,
        }
    }
}

/// Context package.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextPackage {
    /// Original query.
    pub query: String,
    /// Snapshot facts; live database state has not been checked.
    pub snapshot: SnapshotProvenance,
    /// Diagnostics for lexical seed retrieval, before graph expansion.
    pub diagnostics: SearchDiagnostics,
    /// Expansion and output omissions, separate from lexical search coverage.
    pub coverage: ContextCoverage,
    /// Relevant objects.
    pub objects: Vec<ContextObject>,
    /// Relation paths/edges among selected objects.
    pub relation_paths: Vec<String>,
    /// Risk notes.
    pub risks: Vec<String>,
    /// Suggested next tools.
    pub suggested_next_tools: Vec<ToolSuggestion>,
    /// Ranking notes.
    pub ranking_notes: Vec<String>,
    /// Rough token estimate.
    pub estimated_tokens: usize,
}

/// Context expansion and output coverage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ContextCoverage {
    /// Lexical results used as seeds, after the search limit.
    pub seed_objects: usize,
    /// Additional objects reached by one-hop expansion (filters apply to seeds).
    pub neighbor_objects: usize,
    /// Ranked candidates not attempted after reaching the object limit.
    pub omitted_by_limit: usize,
    /// Ranked candidates that did not fit the complete response budget.
    pub omitted_by_budget: usize,
    /// Edges between returned objects that did not fit the response budget.
    pub omitted_relations: usize,
    /// Concrete follow-ups that did not fit the response budget.
    pub omitted_suggestions: usize,
}

/// Context object.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextObject {
    /// Identity within the reported snapshot.
    pub id: String,
    /// Object kind.
    pub kind: String,
    /// Full name.
    pub full_name: String,
    /// Whether the object is a lexical seed or a one-hop graph neighbor.
    pub origin: String,
    /// Lexical terms matched by a seed; neighbors need not match the query.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub matched_terms: Vec<String>,
    /// Existing SQL artifact source, when present.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_path: Option<String>,
    /// Project-owned semantic metadata when configured.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub semantic: Option<serde_json::Value>,
    /// Captured description/comment, omitted when none is available.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub summary: String,
    /// Score.
    pub score: f64,
}

/// Builds AI database context from a snapshot.
pub struct ContextBuilder {
    weights: RankingWeights,
}

impl ContextBuilder {
    /// Creates a new builder.
    #[must_use]
    pub const fn new(weights: RankingWeights) -> Self {
        Self { weights }
    }

    /// Builds a compact context package within the complete JSON token estimate.
    ///
    /// # Errors
    ///
    /// Returns an argument error when the budget cannot hold the response envelope
    /// or any relevant object, and an internal error if serialization fails.
    pub fn build(
        &self,
        snapshot: &DbSnapshot,
        query: &str,
        options: &ContextOptions,
        project_path: &Path,
    ) -> Result<ContextPackage> {
        let search = search_snapshot(
            snapshot,
            query,
            &SearchOptions {
                limit: options.max_objects.saturating_mul(2),
            },
            project_path,
        );
        let objects_by_id = snapshot
            .objects
            .iter()
            .map(|object| (object.id.as_str(), object))
            .collect::<HashMap<_, _>>();
        let seed_terms = search
            .results
            .iter()
            .map(|result| (result.id.as_str(), result.matched_terms.as_slice()))
            .collect::<HashMap<_, _>>();
        let mut scored = search
            .results
            .iter()
            .filter_map(|result| {
                objects_by_id.get(result.id.as_str()).map(|object| {
                    (
                        object.id.as_str(),
                        (*object, result.score * self.weights.text_match),
                    )
                })
            })
            .collect::<BTreeMap<_, _>>();
        self.expand_graph_neighbors(snapshot, &objects_by_id, &mut scored);
        self.apply_statistics_boost(snapshot, &mut scored);
        let workload = WorkloadModel::from_snapshot(snapshot);
        self.apply_workload_boost(&workload, &mut scored);
        let mut package = ContextPackage {
            query: query.to_owned(),
            snapshot: search.snapshot,
            diagnostics: search.diagnostics,
            coverage: ContextCoverage {
                seed_objects: seed_terms.len(),
                neighbor_objects: scored.len().saturating_sub(seed_terms.len()),
                omitted_by_limit: scored.len(),
                ..ContextCoverage::default()
            },
            objects: Vec::new(),
            relation_paths: Vec::new(),
            risks: vec![
                "Read-only context: verify SQL against your local graph before executing changes."
                    .to_owned(),
            ],
            suggested_next_tools: Vec::new(),
            ranking_notes: vec![
                "OR lexical seeds; one-hop neighbors may fall outside filters; follow-ups read latest."
                    .to_owned(),
            ],
            estimated_tokens: 0,
        };
        update_token_estimate(&mut package)?;
        if package.estimated_tokens > options.token_budget {
            return Err(DbGraphError::invalid_argument(format!(
                "context token budget {} cannot fit the response envelope (estimated {} tokens); increase the budget or shorten the query",
                options.token_budget, package.estimated_tokens
            )));
        }
        select_context_objects(
            snapshot,
            &objects_by_id,
            scored,
            &seed_terms,
            options,
            search.suggested_next_tools,
            &mut package,
        )?;
        Ok(package)
    }

    fn expand_graph_neighbors<'a>(
        &self,
        snapshot: &DbSnapshot,
        objects_by_id: &HashMap<&'a str, &'a DbObject>,
        scored: &mut BTreeMap<&'a str, (&'a DbObject, f64)>,
    ) {
        let seed_ids = scored.keys().copied().collect::<HashSet<_>>();
        for edge in &snapshot.edges {
            if seed_ids.contains(edge.from_object_id.as_str())
                || seed_ids.contains(edge.to_object_id.as_str())
            {
                for object_id in [&edge.from_object_id, &edge.to_object_id] {
                    if let Some(object) = objects_by_id.get(object_id.as_str()) {
                        let entry = scored.entry(object.id.as_str()).or_insert((*object, 0.0));
                        entry.1 += 15.0 * self.weights.graph_neighbor;
                    }
                }
            }
        }
    }

    fn apply_statistics_boost(
        &self,
        snapshot: &DbSnapshot,
        scored: &mut BTreeMap<&str, (&DbObject, f64)>,
    ) {
        for profile in &snapshot.table_profiles {
            if let Some(entry) = scored.get_mut(profile.object_id.as_str()) {
                if profile.row_estimate.unwrap_or_default() > 0 {
                    entry.1 += self.weights.statistics;
                }
            }
        }
    }

    fn apply_workload_boost(
        &self,
        workload: &WorkloadModel,
        scored: &mut BTreeMap<&str, (&DbObject, f64)>,
    ) {
        for (object_id, (_, score)) in scored {
            let frequency = workload.frequency(object_id);
            if frequency > 0 {
                *score += f64::from(frequency) * self.weights.workload;
            }
        }
    }
}

fn select_context_objects(
    snapshot: &DbSnapshot,
    objects_by_id: &HashMap<&str, &DbObject>,
    scored: BTreeMap<&str, (&DbObject, f64)>,
    seed_terms: &HashMap<&str, &[String]>,
    options: &ContextOptions,
    suggestions: Vec<ToolSuggestion>,
    package: &mut ContextPackage,
) -> Result<()> {
    let mut objects = scored.into_values().collect::<Vec<_>>();
    objects.sort_by(|left, right| {
        right
            .1
            .partial_cmp(&left.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.0.full_name.cmp(&right.0.full_name))
    });
    let mut suggestions = suggestions.into_iter();
    package.coverage.omitted_suggestions = suggestions.len();
    let mut adjacent = HashMap::<&str, Vec<&DbEdge>>::new();
    for edge in &snapshot.edges {
        adjacent.entry(&edge.from_object_id).or_default().push(edge);
        if edge.from_object_id != edge.to_object_id {
            adjacent.entry(&edge.to_object_id).or_default().push(edge);
        }
    }
    let mut selected_ids = HashSet::new();
    update_token_estimate(package)?;
    for (object, score) in objects {
        if package.objects.len() >= options.max_objects {
            break;
        }
        package.coverage.omitted_by_limit -= 1;
        package.objects.push(context_object(
            object,
            score,
            seed_terms.get(object.id.as_str()).copied(),
        ));
        update_token_estimate(package)?;
        if package.estimated_tokens > options.token_budget {
            package.objects.pop();
            package.coverage.omitted_by_budget += 1;
            update_token_estimate(package)?;
            continue;
        }
        selected_ids.insert(object.id.as_str());
        append_relations(
            package,
            &selected_ids,
            adjacent.get(object.id.as_str()),
            objects_by_id,
            options.token_budget,
        )?;
        if package.objects.len() == 1 {
            // Reserve a concrete follow-up early when it fits, before filling
            // the remainder with objects. Oversized SQL suggestions are optional.
            if let Some(suggestion) = suggestions.next() {
                add_suggestion(package, suggestion, options.token_budget)?;
            }
        }
    }
    if package.objects.is_empty() && package.coverage.omitted_by_budget > 0 {
        return Err(DbGraphError::invalid_argument(format!(
            "context token budget {} cannot fit any relevant object; increase the budget or narrow the query",
            options.token_budget
        )));
    }
    for suggestion in suggestions {
        add_suggestion(package, suggestion, options.token_budget)?;
    }
    // Counter digit changes are part of the same final serialized estimate.
    update_token_estimate(package)?;
    if package.estimated_tokens > options.token_budget {
        return Err(DbGraphError::invalid_argument(
            "context token budget cannot fit coverage metadata; increase the budget",
        ));
    }
    Ok(())
}

fn context_object(object: &DbObject, score: f64, terms: Option<&[String]>) -> ContextObject {
    ContextObject {
        id: object.id.clone(),
        kind: object.kind.as_str().to_owned(),
        full_name: object.full_name.clone(),
        origin: terms
            .map_or("graph_neighbor", |terms| {
                if terms.is_empty() {
                    "filter_match"
                } else {
                    "lexical_match"
                }
            })
            .to_owned(),
        matched_terms: terms.map_or_else(Vec::new, <[String]>::to_vec),
        source_path: object
            .metadata
            .get("sourcePath")
            .and_then(serde_json::Value::as_str)
            .map(ToOwned::to_owned),
        semantic: semantic_metadata(&object.metadata).cloned(),
        summary: object_summary(object),
        score,
    }
}

fn add_suggestion(
    package: &mut ContextPackage,
    suggestion: ToolSuggestion,
    budget: usize,
) -> Result<()> {
    package.suggested_next_tools.push(suggestion);
    package.coverage.omitted_suggestions -= 1;
    update_token_estimate(package)?;
    if package.estimated_tokens > budget {
        package.suggested_next_tools.pop();
        package.coverage.omitted_suggestions += 1;
        update_token_estimate(package)?;
    }
    Ok(())
}

fn append_relations(
    package: &mut ContextPackage,
    selected: &HashSet<&str>,
    adjacent: Option<&Vec<&DbEdge>>,
    objects_by_id: &HashMap<&str, &DbObject>,
    budget: usize,
) -> Result<()> {
    for edge in adjacent.into_iter().flatten() {
        if !selected.contains(edge.from_object_id.as_str())
            || !selected.contains(edge.to_object_id.as_str())
        {
            continue;
        }
        let from = objects_by_id[edge.from_object_id.as_str()];
        let to = objects_by_id[edge.to_object_id.as_str()];
        package.relation_paths.push(format!(
            "{} -{}-> {}",
            from.full_name,
            edge.kind.as_str(),
            to.full_name
        ));
        update_token_estimate(package)?;
        if package.estimated_tokens > budget {
            package.relation_paths.pop();
            package.coverage.omitted_relations += 1;
            update_token_estimate(package)?;
        }
    }
    Ok(())
}

fn object_summary(object: &DbObject) -> String {
    if let Some(description) = semantic_description(&object.metadata) {
        return description.to_owned();
    }
    object
        .metadata
        .get("comment")
        .and_then(|value| value.as_str())
        .map_or_else(String::new, ToOwned::to_owned)
}

fn update_token_estimate(package: &mut ContextPackage) -> Result<()> {
    loop {
        let serialized =
            serde_json::to_string_pretty(package).map_err(|error| DbGraphError::Internal {
                message: format!("failed to serialize context package: {error}"),
            })?;
        let estimate = serialized.len().div_ceil(4);
        if estimate == package.estimated_tokens {
            return Ok(());
        }
        package.estimated_tokens = estimate;
    }
}

#[cfg(test)]
mod tests {
    use crate::context::{ContextBuilder, ContextOptions, RankingWeights};
    use dbgraph_core::model::{DbEdge, DbEdgeKind, DbObject, DbObjectKind, DbSnapshot};
    use std::path::Path;

    #[test]
    fn context_builder_returns_ranked_refund_payment_order_context_with_budget() {
        let snapshot = sample_snapshot();
        let context = ContextBuilder::new(RankingWeights::default())
            .build(
                &snapshot,
                "refund payment order",
                &ContextOptions {
                    token_budget: 500,
                    max_objects: 5,
                },
                Path::new("."),
            )
            .expect("context should fit the complete JSON budget");

        let names = context
            .objects
            .iter()
            .map(|object| object.full_name.as_str())
            .collect::<Vec<_>>();

        assert!(names.contains(&"public.refunds"));
        assert!(names.contains(&"public.payments"));
        assert!(names.contains(&"public.orders"));
        assert!(context.estimated_tokens <= 500);
        assert_ne!(context.ranking_notes, [] as [String; 0]);
    }

    #[test]
    fn context_object_includes_semantic_metadata_when_present() {
        let mut snapshot = sample_snapshot();
        let orders = snapshot
            .objects
            .iter_mut()
            .find(|object| object.full_name == "public.orders")
            .expect("orders table should exist");
        orders.metadata.insert(
            "semantic".to_owned(),
            serde_json::json!({
                "description": "Commercial order aggregate",
                "owner": "commerce",
                "certified": true
            }),
        );

        let context = ContextBuilder::new(RankingWeights::default())
            .build(
                &snapshot,
                "orders",
                &ContextOptions {
                    token_budget: 500,
                    max_objects: 3,
                },
                Path::new("."),
            )
            .expect("semantic metadata should fit the budget");

        let orders = context
            .objects
            .iter()
            .find(|object| object.full_name == "public.orders")
            .expect("orders should be selected");

        assert_eq!(
            orders
                .semantic
                .as_ref()
                .and_then(|metadata| metadata.get("description"))
                .and_then(serde_json::Value::as_str),
            Some("Commercial order aggregate")
        );
        assert!(orders.summary.contains("Commercial order aggregate"));
    }

    #[test]
    fn context_budget_counts_semantics_and_keeps_smaller_relevant_candidates() {
        let mut snapshot = sample_snapshot();
        snapshot.objects[0].metadata.insert(
            "semantic".to_owned(),
            serde_json::json!({"allowedValues": (0..500).map(|value| format!("state_{value}")).collect::<Vec<_>>()}),
        );

        let context = ContextBuilder::new(RankingWeights::default())
            .build(
                &snapshot,
                "refunds payment",
                &ContextOptions {
                    token_budget: 500,
                    max_objects: 5,
                },
                Path::new("."),
            )
            .expect("smaller relevant objects should still fit");

        assert!(!context
            .objects
            .iter()
            .any(|object| object.full_name == "public.refunds"));
        assert!(context.coverage.omitted_by_budget > 0);
        assert!(context
            .objects
            .iter()
            .any(|object| object.full_name == "public.payments"));
        let json = serde_json::to_string_pretty(&context).expect("context JSON");
        assert!(json.len() <= 500 * 4);
        assert!(context.estimated_tokens * 4 >= json.len());
        assert!(context.estimated_tokens <= 500);
    }

    #[test]
    fn context_distinguishes_no_matches_from_insufficient_budget() {
        let mut snapshot = sample_snapshot();
        let builder = ContextBuilder::new(RankingWeights::default());
        let options = ContextOptions {
            token_budget: 500,
            max_objects: 5,
        };
        let empty = builder
            .build(&snapshot, "不存在", &options, Path::new("."))
            .expect("no matches is valid");
        assert_eq!(empty.objects, [] as [super::ContextObject; 0]);
        assert_eq!(empty.diagnostics.status, "no_term_match");
        assert!(empty.suggested_next_tools.is_empty());
        assert!(builder
            .build(
                &snapshot,
                "refund",
                &ContextOptions {
                    token_budget: 1,
                    max_objects: 5
                },
                Path::new("."),
            )
            .is_err());
        assert!(builder
            .build(
                &snapshot,
                &"very_long_query ".repeat(500),
                &options,
                Path::new(".")
            )
            .is_err());

        snapshot.objects.truncate(1);
        snapshot.edges.clear();
        snapshot.objects[0].metadata.insert(
            "semantic".to_owned(),
            serde_json::json!({"description": "refund ".repeat(1000)}),
        );
        assert!(builder
            .build(&snapshot, "refund", &options, Path::new("."))
            .is_err());
    }

    #[test]
    fn context_distinguishes_seed_filters_neighbors_and_object_limits() {
        let snapshot = sample_snapshot();
        let builder = ContextBuilder::new(RankingWeights::default());
        let mut options = ContextOptions {
            token_budget: 1200,
            max_objects: 3,
        };
        let context = builder
            .build(
                &snapshot,
                "kind:table name:refund",
                &options,
                Path::new("."),
            )
            .expect("context");
        assert_eq!(context.snapshot.id, snapshot.id);
        assert_eq!(
            context.snapshot.created_at_unix_ms,
            snapshot.created_at_unix_ms
        );
        assert_eq!(context.snapshot.live_state, "not_checked");
        assert_eq!(context.diagnostics.coverage.candidate_objects, 1);
        assert_eq!(context.coverage.seed_objects, 1);
        assert_eq!(context.coverage.neighbor_objects, 1);
        assert!(context.objects.iter().any(
            |object| object.full_name == "public.payments" && object.origin == "graph_neighbor"
        ));
        assert!(context
            .objects
            .iter()
            .any(|object| object.full_name == "public.refunds" && object.origin == "filter_match"));

        options.max_objects = 1;
        let limited = builder
            .build(
                &snapshot,
                "kind:table name:refund",
                &options,
                Path::new("."),
            )
            .expect("limited context");
        assert_eq!(limited.objects.len(), 1);
        assert_eq!(limited.coverage.omitted_by_limit, 1);
        assert_eq!(limited.coverage.omitted_by_budget, 0);
    }

    #[test]
    fn context_keeps_sql_source_but_omits_oversized_sql_follow_up_within_budget() {
        let mut snapshot = DbSnapshot::new("sql-snapshot", "sqlite", "app", 17);
        let mut query = DbObject::new("query:sql/order.sql", DbObjectKind::Query, "sql.order.sql");
        query
            .metadata
            .insert("sourcePath".to_owned(), serde_json::json!("sql/order.sql"));
        query.metadata.insert(
            "normalizedSql".to_owned(),
            serde_json::json!(format!(
                "SELECT {} FROM orders",
                vec!["id"; 1000].join(", ")
            )),
        );
        snapshot.objects.push(query);
        let context = ContextBuilder::new(RankingWeights::default())
            .build(
                &snapshot,
                "kind:query",
                &ContextOptions::default(),
                Path::new("different-project"),
            )
            .expect("small context without huge SQL suggestion");
        assert_eq!(
            context.objects[0].source_path.as_deref(),
            Some("sql/order.sql")
        );
        assert_eq!(context.coverage.omitted_suggestions, 1);
        assert!(context.suggested_next_tools.is_empty());
        let serialized = serde_json::to_string_pretty(&context).expect("JSON");
        assert!(serialized.len() <= ContextOptions::default().token_budget * 4);
        assert_eq!(
            serde_json::from_str::<super::ContextPackage>(&serialized).expect("round trip"),
            context
        );
    }

    #[test]
    fn context_bounds_dense_relations_and_counts_omitted_edges() {
        let mut snapshot = sample_snapshot();
        snapshot.objects.truncate(2);
        snapshot.edges = (0..200)
            .map(|index| {
                DbEdge::explicit(
                    format!("edge:{index}"),
                    DbEdgeKind::References,
                    "table:refunds",
                    "table:payments",
                )
            })
            .collect();
        let context = ContextBuilder::new(RankingWeights::default())
            .build(
                &snapshot,
                "refund",
                &ContextOptions::default(),
                Path::new("."),
            )
            .expect("dense graph should fit with explicit omissions");
        assert_eq!(context.objects.len(), 2);
        assert!(!context.relation_paths.is_empty());
        assert!(context.coverage.omitted_relations > 0);
        assert_eq!(
            context.relation_paths.len() + context.coverage.omitted_relations,
            snapshot.edges.len()
        );
        assert!(
            serde_json::to_string_pretty(&context).expect("JSON").len()
                <= ContextOptions::default().token_budget * 4
        );
    }

    fn sample_snapshot() -> DbSnapshot {
        let mut snapshot = DbSnapshot::new("s1", "postgres", "app", 1);
        for (id, name, comment) in [
            ("table:refunds", "public.refunds", "refund request records"),
            (
                "table:payments",
                "public.payments",
                "payment ledger records",
            ),
            ("table:orders", "public.orders", "customer order records"),
            ("table:users", "public.users", "login records"),
        ] {
            let mut object = DbObject::new(id, DbObjectKind::Table, name);
            object.metadata.insert(
                "comment".to_owned(),
                serde_json::Value::String(comment.to_owned()),
            );
            snapshot.objects.push(object);
        }
        snapshot.edges.push(DbEdge::explicit(
            "edge:refunds.payments",
            DbEdgeKind::References,
            "table:refunds",
            "table:payments",
        ));
        snapshot.edges.push(DbEdge::explicit(
            "edge:payments.orders",
            DbEdgeKind::References,
            "table:payments",
            "table:orders",
        ));
        snapshot
    }
}
