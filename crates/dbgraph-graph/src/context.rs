//! Context candidate retrieval and formatting.

use std::collections::{BTreeMap, HashMap, HashSet};

use dbgraph_core::model::{DbObject, DbSnapshot};
use dbgraph_core::semantics::{semantic_description, semantic_metadata};
use dbgraph_core::{DbGraphError, Result};
use serde::{Deserialize, Serialize};

use crate::search::{extract_search_terms, search_snapshot, SearchOptions};
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
    /// Relevant objects.
    pub objects: Vec<ContextObject>,
    /// Relation paths/edges among selected objects.
    pub relation_paths: Vec<String>,
    /// Risk notes.
    pub risks: Vec<String>,
    /// Suggested next tools.
    pub suggested_next_tools: Vec<String>,
    /// Ranking notes.
    pub ranking_notes: Vec<String>,
    /// Rough token estimate.
    pub estimated_tokens: usize,
}

/// Context object.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextObject {
    /// Object kind.
    pub kind: String,
    /// Full name.
    pub full_name: String,
    /// Project-owned semantic metadata when configured.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub semantic: Option<serde_json::Value>,
    /// Summary.
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
    ) -> Result<ContextPackage> {
        let terms = extract_search_terms(query);
        let mut scored = self.seed_scored_objects(snapshot, query, options.max_objects);
        self.expand_graph_neighbors(snapshot, &mut scored);
        self.apply_statistics_boost(snapshot, &mut scored);
        let workload = WorkloadModel::from_snapshot(snapshot);
        self.apply_workload_boost(&workload, &mut scored);
        let mut package = ContextPackage {
            query: query.to_owned(),
            objects: Vec::new(),
            relation_paths: Vec::new(),
            risks: vec![
                "Read-only context: verify SQL against your local graph before executing changes."
                    .to_owned(),
            ],
            suggested_next_tools: vec![
                "dbgraph table <table>".to_owned(),
                "dbgraph relations <object>".to_owned(),
                "dbgraph validate-sql --sql <SQL>".to_owned(),
            ],
            ranking_notes: vec![format!("terms: {}", terms.join(", "))],
            estimated_tokens: 0,
        };
        update_token_estimate(&mut package)?;
        if package.estimated_tokens > options.token_budget {
            return Err(DbGraphError::invalid_argument(format!(
                "context token budget {} cannot fit the response envelope (estimated {} tokens); increase the budget or shorten the query",
                options.token_budget, package.estimated_tokens
            )));
        }
        select_context_objects(snapshot, scored, options, &mut package)?;
        Ok(package)
    }

    fn seed_scored_objects(
        &self,
        snapshot: &DbSnapshot,
        query: &str,
        max_objects: usize,
    ) -> BTreeMap<String, (DbObject, f64)> {
        let mut scored = BTreeMap::<String, (DbObject, f64)>::new();
        for result in search_snapshot(
            snapshot,
            query,
            &SearchOptions {
                limit: max_objects * 2,
            },
        ) {
            if let Some(object) = snapshot
                .objects
                .iter()
                .find(|object| object.id == result.id)
            {
                scored.insert(
                    object.id.clone(),
                    (object.clone(), result.score * self.weights.text_match),
                );
            }
        }
        scored
    }

    fn expand_graph_neighbors(
        &self,
        snapshot: &DbSnapshot,
        scored: &mut BTreeMap<String, (DbObject, f64)>,
    ) {
        let seed_ids = scored.keys().cloned().collect::<HashSet<_>>();
        for edge in &snapshot.edges {
            if seed_ids.contains(&edge.from_object_id) || seed_ids.contains(&edge.to_object_id) {
                for object_id in [&edge.from_object_id, &edge.to_object_id] {
                    if let Some(object) = snapshot
                        .objects
                        .iter()
                        .find(|object| object.id == *object_id)
                    {
                        let entry = scored
                            .entry(object.id.clone())
                            .or_insert((object.clone(), 0.0));
                        entry.1 += 15.0 * self.weights.graph_neighbor;
                    }
                }
            }
        }
    }

    fn apply_statistics_boost(
        &self,
        snapshot: &DbSnapshot,
        scored: &mut BTreeMap<String, (DbObject, f64)>,
    ) {
        for profile in &snapshot.table_profiles {
            if let Some(entry) = scored.get_mut(&profile.object_id) {
                if profile.row_estimate.unwrap_or_default() > 0 {
                    entry.1 += self.weights.statistics;
                }
            }
        }
    }

    fn apply_workload_boost(
        &self,
        workload: &WorkloadModel,
        scored: &mut BTreeMap<String, (DbObject, f64)>,
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
    scored: BTreeMap<String, (DbObject, f64)>,
    options: &ContextOptions,
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
    let object_names = snapshot
        .objects
        .iter()
        .map(|object| (object.id.as_str(), object.full_name.as_str()))
        .collect::<HashMap<_, _>>();

    let mut omitted_for_budget = false;
    for (object, score) in objects {
        if package.objects.len() >= options.max_objects {
            break;
        }
        let summary = object_summary(&object);
        let previous_tokens = package.estimated_tokens;
        let previous_paths = std::mem::take(&mut package.relation_paths);
        package.objects.push(ContextObject {
            kind: object.kind.as_str().to_owned(),
            full_name: object.full_name,
            semantic: semantic_metadata(&object.metadata).cloned(),
            summary,
            score,
        });
        package.relation_paths = relation_paths(snapshot, &object_names, &package.objects);
        update_token_estimate(package)?;
        if package.estimated_tokens > options.token_budget {
            package.objects.pop();
            package.relation_paths = previous_paths;
            package.estimated_tokens = previous_tokens;
            omitted_for_budget = true;
        }
    }
    if package.objects.is_empty() && omitted_for_budget {
        return Err(DbGraphError::invalid_argument(format!(
            "context token budget {} cannot fit any relevant object; increase the budget or narrow the query",
            options.token_budget
        )));
    }
    Ok(())
}

fn relation_paths(
    snapshot: &DbSnapshot,
    object_names: &HashMap<&str, &str>,
    context_objects: &[ContextObject],
) -> Vec<String> {
    let selected = context_objects
        .iter()
        .map(|object| object.full_name.as_str())
        .collect::<HashSet<_>>();
    snapshot
        .edges
        .iter()
        .filter_map(|edge| {
            let from = object_names.get(edge.from_object_id.as_str())?;
            let to = object_names.get(edge.to_object_id.as_str())?;
            (selected.contains(from) && selected.contains(to))
                .then(|| format!("{from} -{}-> {to}", edge.kind.as_str()))
        })
        .collect()
}

fn object_summary(object: &DbObject) -> String {
    if let Some(description) = semantic_description(&object.metadata) {
        return description.to_owned();
    }
    object
        .metadata
        .get("comment")
        .and_then(|value| value.as_str())
        .map_or_else(
            || format!("{} {}", object.kind.as_str(), object.full_name),
            ToOwned::to_owned,
        )
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
        assert!(!context.ranking_notes.is_empty());
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
            )
            .expect("smaller relevant objects should still fit");

        assert!(!context
            .objects
            .iter()
            .any(|object| object.full_name == "public.refunds"));
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
            .build(&snapshot, "不存在", &options)
            .expect("no matches is valid");
        assert!(empty.objects.is_empty());
        assert!(builder
            .build(
                &snapshot,
                "refund",
                &ContextOptions {
                    token_budget: 1,
                    max_objects: 5
                }
            )
            .is_err());
        assert!(builder
            .build(&snapshot, &"very_long_query ".repeat(500), &options)
            .is_err());

        snapshot.objects.truncate(1);
        snapshot.edges.clear();
        snapshot.objects[0].metadata.insert(
            "semantic".to_owned(),
            serde_json::json!({"description": "refund ".repeat(1000)}),
        );
        assert!(builder.build(&snapshot, "refund", &options).is_err());
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
