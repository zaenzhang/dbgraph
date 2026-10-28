//! Compact snapshot provenance and evidence-backed follow-up calls.

use std::path::Path;

use dbgraph_core::model::{DbObject, DbObjectKind, DbSnapshot};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Identity of the local facts used to produce a report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotProvenance {
    /// Published snapshot identity.
    pub id: String,
    /// Capturing database provider.
    pub provider: String,
    /// Database name reported by the provider.
    pub database_name: String,
    /// Capture time, not a live schema verification time.
    pub created_at_unix_ms: u64,
    /// Whether current database state was checked by this local query.
    pub live_state: String,
}

impl SnapshotProvenance {
    /// Reuses identity and capture facts without contacting the database.
    #[must_use]
    pub fn from_snapshot(snapshot: &DbSnapshot) -> Self {
        Self {
            id: snapshot.id.clone(),
            provider: snapshot.provider.clone(),
            database_name: snapshot.database_name.clone(),
            created_at_unix_ms: snapshot.created_at_unix_ms,
            live_state: "not_checked".to_owned(),
        }
    }
}

/// A directly callable existing MCP tool, scoped to the queried project.
///
/// The call uses the latest snapshot at execution time; this does not pin it to
/// the report's snapshot identity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolSuggestion {
    /// Existing tool name.
    pub tool: String,
    /// Complete concrete arguments, including project scope.
    pub arguments: Value,
    /// Meaning of this follow-up, not a claim that the task is answered.
    pub reason: String,
}

/// Suggests at most two expansions from the supplied ranked snapshot objects.
///
/// Callers provide actual returned candidates. No object names or SQL are
/// generated, and an empty candidate set produces no suggestion.
#[must_use]
pub fn object_suggestions<'a>(
    snapshot: &DbSnapshot,
    objects: impl IntoIterator<Item = &'a DbObject>,
    project_path: &Path,
) -> Vec<ToolSuggestion> {
    objects
        .into_iter()
        .take(2)
        .map(|object| object_suggestion(snapshot, object, project_path))
        .collect()
}

fn object_suggestion(
    snapshot: &DbSnapshot,
    object: &DbObject,
    project_path: &Path,
) -> ToolSuggestion {
    if object.kind == DbObjectKind::Table {
        return ToolSuggestion {
            tool: "dbgraph_table".to_owned(),
            arguments: json!({"projectPath": project_path, "table": object.full_name}),
            reason: "inspect_snapshot_table".to_owned(),
        };
    }
    if object.kind == DbObjectKind::Query {
        let dialect = match snapshot.provider.as_str() {
            "postgres" => Some("postgres"),
            "sqlite" => Some("generic"),
            _ => None,
        };
        if let (Some(dialect), Some(sql)) = (
            dialect,
            object
                .metadata
                .get("normalizedSql")
                .and_then(Value::as_str)
                .filter(|sql| !sql.trim().is_empty()),
        ) {
            return ToolSuggestion {
                tool: "dbgraph_validate_sql".to_owned(),
                arguments: json!({"projectPath": project_path, "sql": sql, "dialect": dialect}),
                reason: "validate_snapshot_sql".to_owned(),
            };
        }
    }
    ToolSuggestion {
        tool: "dbgraph_relations".to_owned(),
        arguments: json!({"projectPath": project_path, "object": object.id}),
        reason: "inspect_snapshot_object_relations".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suggestions_use_real_evidence_and_preserve_project_scope() {
        let project = Path::new("different-project");
        let mut snapshot = DbSnapshot::new("s1", "sqlite", "app", 123);
        let table = DbObject::new("table:orders", DbObjectKind::Table, "main.orders");
        let mut query = DbObject::new("query:sql/orders.sql", DbObjectKind::Query, "sql.orders");
        query.metadata.insert(
            "normalizedSql".to_owned(),
            json!("SELECT id FROM main.orders"),
        );
        snapshot.objects = vec![table, query];
        let suggestions = object_suggestions(&snapshot, &snapshot.objects, project);

        assert_eq!(suggestions[0].tool, "dbgraph_table");
        assert_eq!(suggestions[0].arguments["table"], "main.orders");
        assert_eq!(suggestions[1].tool, "dbgraph_validate_sql");
        assert_eq!(suggestions[1].arguments["dialect"], "generic");
        assert_eq!(
            suggestions[1].arguments["sql"],
            snapshot.objects[1].metadata["normalizedSql"]
        );
        for suggestion in suggestions {
            assert_eq!(suggestion.arguments["projectPath"], json!(project));
        }
        assert_eq!(
            SnapshotProvenance::from_snapshot(&snapshot).live_state,
            "not_checked"
        );
    }

    #[test]
    fn no_candidates_or_sql_does_not_invent_follow_up_parameters() {
        let mut snapshot = DbSnapshot::new("s1", "postgres", "app", 1);
        assert!(object_suggestions(&snapshot, &snapshot.objects, Path::new(".")).is_empty());
        snapshot.objects.push(DbObject::new(
            "query:empty",
            DbObjectKind::Query,
            "sql.empty",
        ));
        let suggestions = object_suggestions(&snapshot, &snapshot.objects, Path::new("."));
        assert_eq!(suggestions[0].tool, "dbgraph_relations");
        assert_eq!(suggestions[0].arguments["object"], "query:empty");
        assert!(suggestions[0].arguments.get("sql").is_none());
    }

    #[test]
    fn suggestions_are_bounded_and_use_ids_for_non_tables() {
        let mut snapshot = DbSnapshot::new("s1", "postgres", "app", 1);
        for schema in ["tenant_a", "tenant_b", "tenant_c"] {
            snapshot.objects.push(DbObject::new(
                format!("column:{schema}.orders.id"),
                DbObjectKind::Column,
                format!("{schema}.orders.id"),
            ));
        }
        let suggestions = object_suggestions(&snapshot, &snapshot.objects, Path::new("."));
        assert_eq!(suggestions.len(), 2);
        for (suggestion, object) in suggestions.iter().zip(&snapshot.objects) {
            assert_eq!(suggestion.tool, "dbgraph_relations");
            assert_eq!(suggestion.arguments["object"], object.id);
        }
    }
}
