//! Canonical database snapshot, object, edge, and profile model.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Stable JSON object for provider-specific metadata.
pub type Metadata = BTreeMap<String, Value>;

/// Indicates whether a provider can supply a capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityStatus {
    /// The provider supports the capability.
    Supported,
    /// The provider does not support the capability.
    Unsupported,
    /// The provider did not report whether this capability exists.
    Unknown,
}

/// Capability matrix embedded into snapshots.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderCapabilities {
    /// Schema, table, and column metadata.
    pub schema_metadata: CapabilityStatus,
    /// Primary key, foreign key, unique, and check constraints.
    pub constraints: CapabilityStatus,
    /// Index metadata.
    pub indexes: CapabilityStatus,
    /// View metadata.
    pub views: CapabilityStatus,
    /// Function or procedure metadata.
    pub routines: CapabilityStatus,
    /// Trigger metadata.
    pub triggers: CapabilityStatus,
    /// Catalog or optimizer statistics.
    pub statistics: CapabilityStatus,
    /// Optional masked row sampling.
    pub sampling: CapabilityStatus,
}

impl Default for ProviderCapabilities {
    fn default() -> Self {
        Self {
            schema_metadata: CapabilityStatus::Unknown,
            constraints: CapabilityStatus::Unknown,
            indexes: CapabilityStatus::Unknown,
            views: CapabilityStatus::Unknown,
            routines: CapabilityStatus::Unknown,
            triggers: CapabilityStatus::Unknown,
            statistics: CapabilityStatus::Unknown,
            sampling: CapabilityStatus::Unsupported,
        }
    }
}

/// Canonical database object kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DbObjectKind {
    /// Database root object.
    Database,
    /// Database schema or namespace.
    Schema,
    /// Table.
    Table,
    /// Column.
    Column,
    /// Primary key constraint.
    PrimaryKey,
    /// Foreign key constraint.
    ForeignKey,
    /// Unique constraint.
    UniqueConstraint,
    /// Check constraint.
    CheckConstraint,
    /// Index.
    Index,
    /// View.
    View,
    /// Materialized view.
    MaterializedView,
    /// Function.
    Function,
    /// Procedure.
    Procedure,
    /// Trigger.
    Trigger,
    /// Enum type.
    Enum,
    /// Sequence.
    Sequence,
    /// Query.
    Query,
    /// Migration file or migration record.
    Migration,
    /// SQL artifact discovered in source files.
    SqlArtifact,
}

impl DbObjectKind {
    /// Returns the stable storage string.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Database => "database",
            Self::Schema => "schema",
            Self::Table => "table",
            Self::Column => "column",
            Self::PrimaryKey => "primary_key",
            Self::ForeignKey => "foreign_key",
            Self::UniqueConstraint => "unique_constraint",
            Self::CheckConstraint => "check_constraint",
            Self::Index => "index",
            Self::View => "view",
            Self::MaterializedView => "materialized_view",
            Self::Function => "function",
            Self::Procedure => "procedure",
            Self::Trigger => "trigger",
            Self::Enum => "enum",
            Self::Sequence => "sequence",
            Self::Query => "query",
            Self::Migration => "migration",
            Self::SqlArtifact => "sql_artifact",
        }
    }
}

/// Canonical database edge kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DbEdgeKind {
    /// Object contains another object.
    Contains,
    /// Table has column.
    HasColumn,
    /// Object has constraint.
    HasConstraint,
    /// Object has index.
    HasIndex,
    /// Explicit reference, usually a foreign key.
    References,
    /// Dependency relation.
    DependsOn,
    /// View uses object.
    UsedByView,
    /// Trigger relation.
    TriggeredBy,
    /// Query reads from object.
    ReadsFrom,
    /// Query writes to object.
    WritesTo,
    /// Query joins on object.
    JoinsOn,
    /// Query filters by object.
    FiltersBy,
    /// Query groups by object.
    GroupsBy,
    /// Query orders by object.
    OrdersBy,
    /// Inferred possible reference.
    InferredReference,
}

impl DbEdgeKind {
    /// Returns the stable storage string.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Contains => "contains",
            Self::HasColumn => "has_column",
            Self::HasConstraint => "has_constraint",
            Self::HasIndex => "has_index",
            Self::References => "references",
            Self::DependsOn => "depends_on",
            Self::UsedByView => "used_by_view",
            Self::TriggeredBy => "triggered_by",
            Self::ReadsFrom => "reads_from",
            Self::WritesTo => "writes_to",
            Self::JoinsOn => "joins_on",
            Self::FiltersBy => "filters_by",
            Self::GroupsBy => "groups_by",
            Self::OrdersBy => "orders_by",
            Self::InferredReference => "inferred_reference",
        }
    }
}

/// Evidence attached to explicit or inferred relations.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Evidence {
    /// Evidence source, such as `postgres.catalog` or `naming_rule`.
    pub source: String,
    /// Human-readable evidence detail.
    pub detail: String,
}

/// Optional table metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TableMetadata {
    /// Table type, for example base table or partitioned table.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub table_type: Option<String>,
    /// Table comment.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
}

/// Optional column metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ColumnMetadata {
    /// Native database type.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_type: Option<String>,
    /// Normalized type family.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_type_family: Option<String>,
    /// Whether the column is nullable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nullable: Option<bool>,
    /// Default expression.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
    /// Column comment.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
}

/// Optional index metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct IndexMetadata {
    /// Whether the index is unique.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unique: Option<bool>,
    /// Indexed columns.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub columns: Vec<String>,
    /// Index expression.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expression: Option<String>,
}

/// Optional constraint metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ConstraintMetadata {
    /// Constraint columns.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub columns: Vec<String>,
    /// Referenced table for foreign keys.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub referenced_table: Option<String>,
    /// Referenced columns for foreign keys.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub referenced_columns: Vec<String>,
}

/// Canonical database object.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DbObject {
    /// Stable object id within the snapshot.
    pub id: String,
    /// Object kind.
    pub kind: DbObjectKind,
    /// Short name.
    pub name: String,
    /// Fully qualified name.
    pub full_name: String,
    /// Schema name when applicable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema_name: Option<String>,
    /// Table name when applicable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub table_name: Option<String>,
    /// Column name when applicable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub column_name: Option<String>,
    /// Table metadata.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub table: Option<TableMetadata>,
    /// Column metadata.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub column: Option<ColumnMetadata>,
    /// Index metadata.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index: Option<IndexMetadata>,
    /// Constraint metadata.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub constraint: Option<ConstraintMetadata>,
    /// Provider-specific metadata.
    #[serde(default, skip_serializing_if = "Metadata::is_empty")]
    pub metadata: Metadata,
}

impl DbObject {
    /// Creates a minimal object.
    #[must_use]
    pub fn new(id: impl Into<String>, kind: DbObjectKind, full_name: impl Into<String>) -> Self {
        let full_name = full_name.into();
        let name = full_name
            .rsplit('.')
            .next()
            .map_or_else(|| full_name.clone(), ToOwned::to_owned);
        Self {
            id: id.into(),
            kind,
            name,
            full_name,
            schema_name: None,
            table_name: None,
            column_name: None,
            table: None,
            column: None,
            index: None,
            constraint: None,
            metadata: Metadata::new(),
        }
    }
}

/// Canonical graph edge.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DbEdge {
    /// Stable edge id.
    pub id: String,
    /// Edge kind.
    pub kind: DbEdgeKind,
    /// Source object id.
    pub from_object_id: String,
    /// Target object id.
    pub to_object_id: String,
    /// Confidence score from `0.0` to `1.0`.
    pub confidence: f64,
    /// Evidence records.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<Evidence>,
    /// Provider-specific metadata.
    #[serde(default, skip_serializing_if = "Metadata::is_empty")]
    pub metadata: Metadata,
}

impl DbEdge {
    /// Creates an explicit edge with confidence `1.0`.
    #[must_use]
    pub fn explicit(
        id: impl Into<String>,
        kind: DbEdgeKind,
        from_object_id: impl Into<String>,
        to_object_id: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            kind,
            from_object_id: from_object_id.into(),
            to_object_id: to_object_id.into(),
            confidence: 1.0,
            evidence: Vec::new(),
            metadata: Metadata::new(),
        }
    }
}

/// Table profile.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TableProfile {
    /// Object id for the table.
    pub object_id: String,
    /// Estimated or exact row count.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub row_estimate: Option<i64>,
    /// Row count source/kind.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub row_count_kind: Option<String>,
    /// Approximate size in bytes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<i64>,
    /// Profile metadata.
    #[serde(default, skip_serializing_if = "Metadata::is_empty")]
    pub profile: Metadata,
}

/// Column profile.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ColumnProfile {
    /// Object id for the column.
    pub object_id: String,
    /// Normalized type family.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_type_family: Option<String>,
    /// Null fraction.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub null_fraction: Option<f64>,
    /// Distinct estimate.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub distinct_estimate: Option<f64>,
    /// PII score.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pii_score: Option<f64>,
    /// Profile metadata.
    #[serde(default, skip_serializing_if = "Metadata::is_empty")]
    pub profile: Metadata,
}

/// Canonical database snapshot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DbSnapshot {
    /// Stable snapshot id.
    pub id: String,
    /// Provider id.
    pub provider: String,
    /// Database name.
    pub database_name: String,
    /// Creation timestamp in Unix milliseconds.
    pub created_at_unix_ms: u64,
    /// Stable schema hash.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema_hash: Option<String>,
    /// Provider capabilities.
    pub capabilities: ProviderCapabilities,
    /// Objects.
    #[serde(default)]
    pub objects: Vec<DbObject>,
    /// Edges.
    #[serde(default)]
    pub edges: Vec<DbEdge>,
    /// Table profiles.
    #[serde(default)]
    pub table_profiles: Vec<TableProfile>,
    /// Column profiles.
    #[serde(default)]
    pub column_profiles: Vec<ColumnProfile>,
    /// Provider-specific snapshot metadata.
    #[serde(default, skip_serializing_if = "Metadata::is_empty")]
    pub metadata: Metadata,
}

impl DbSnapshot {
    /// Creates an empty snapshot.
    #[must_use]
    pub fn new(
        id: impl Into<String>,
        provider: impl Into<String>,
        database_name: impl Into<String>,
        created_at_unix_ms: u64,
    ) -> Self {
        Self {
            id: id.into(),
            provider: provider.into(),
            database_name: database_name.into(),
            created_at_unix_ms,
            schema_hash: None,
            capabilities: ProviderCapabilities::default(),
            objects: Vec::new(),
            edges: Vec::new(),
            table_profiles: Vec::new(),
            column_profiles: Vec::new(),
            metadata: Metadata::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_serializes_explicit_and_inferred_edges() {
        let mut snapshot = DbSnapshot::new("s1", "postgres", "app", 1);
        snapshot.objects.push(DbObject::new(
            "orders",
            DbObjectKind::Table,
            "public.orders",
        ));
        snapshot
            .objects
            .push(DbObject::new("users", DbObjectKind::Table, "public.users"));
        snapshot.edges.push(DbEdge::explicit(
            "e1",
            DbEdgeKind::References,
            "orders",
            "users",
        ));
        snapshot.edges.push(DbEdge {
            id: "e2".to_owned(),
            kind: DbEdgeKind::InferredReference,
            from_object_id: "orders".to_owned(),
            to_object_id: "users".to_owned(),
            confidence: 0.82,
            evidence: vec![Evidence {
                source: "naming_rule".to_owned(),
                detail: "orders.user_id -> users.id".to_owned(),
            }],
            metadata: Metadata::new(),
        });

        let json = serde_json::to_string_pretty(&snapshot).expect("snapshot should serialize");

        assert!(json.contains("\"references\""));
        assert!(json.contains("\"inferred_reference\""));
        assert!(json.contains("\"confidence\": 0.82"));
    }

    #[test]
    fn capabilities_can_mark_unknown_and_unsupported() {
        let capabilities = ProviderCapabilities::default();
        let json = serde_json::to_string(&capabilities).expect("capabilities should serialize");

        assert!(json.contains("unknown"));
        assert!(json.contains("unsupported"));
    }
}
