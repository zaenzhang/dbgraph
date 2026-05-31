//! Analysis finding construction and rule metadata.

use dbgraph_core::model::DbObject;

use super::{AnalysisFinding, AnalysisScope, FindingSeverity};

pub(super) fn finding(
    scope: AnalysisScope,
    severity: FindingSeverity,
    rule_id: &str,
    object: &DbObject,
    message: &str,
    evidence: String,
    related_objects: Vec<String>,
) -> AnalysisFinding {
    let metadata = rule_metadata(rule_id);
    let fingerprint = finding_fingerprint(rule_id, &object.full_name, &related_objects);
    AnalysisFinding {
        scope,
        severity,
        rule_id: rule_id.to_owned(),
        object: object.full_name.clone(),
        message: message.to_owned(),
        evidence,
        title: metadata.title.to_owned(),
        description: metadata.description.to_owned(),
        impact: metadata.impact.to_owned(),
        suggested_fix: metadata.suggested_fix.to_owned(),
        confidence: metadata.confidence,
        tags: metadata.tags.iter().map(|tag| (*tag).to_owned()).collect(),
        related_objects,
        fingerprint,
    }
}

fn finding_fingerprint(rule_id: &str, object: &str, related_objects: &[String]) -> String {
    let mut related = related_objects.to_vec();
    related.sort();
    related.dedup();
    format!("{rule_id}|{object}|{}", related.join(","))
}

struct RuleMetadata {
    title: &'static str,
    description: &'static str,
    impact: &'static str,
    suggested_fix: &'static str,
    confidence: f64,
    tags: &'static [&'static str],
}

fn rule_metadata(rule_id: &str) -> RuleMetadata {
    if let Some(metadata) = data_rule_metadata(rule_id) {
        return metadata;
    }
    match rule_id {
        "risk.sensitive_column" => RuleMetadata {
            title: "Sensitive column detected",
            description: "Column profiling indicates likely PII or secret-bearing data.",
            impact: "Uncontrolled reads can create privacy, compliance, or credential exposure risk.",
            suggested_fix: "Review access controls, mask or redact this value in downstream outputs, and avoid broad SELECT * projections.",
            confidence: 0.9,
            tags: &["risk", "pii", "privacy"],
        },
        "risk.query_reads_sensitive_column" => RuleMetadata {
            title: "SQL reads sensitive column",
            description: "A SQL artifact references a column with elevated PII score.",
            impact: "Application or analytics code may propagate sensitive data beyond its intended boundary.",
            suggested_fix: "Project only required columns, mask sensitive values where possible, and review the related SQL artifact.",
            confidence: 0.85,
            tags: &["risk", "pii", "sql"],
        },
        "risk.select_star" => RuleMetadata {
            title: "SELECT * query detected",
            description: "The SQL artifact selects all columns from at least one source.",
            impact: "Future schema changes can silently expose new columns or increase query cost.",
            suggested_fix: "Replace SELECT * with explicit column projection and exclude sensitive or unused fields.",
            confidence: 0.8,
            tags: &["risk", "sql", "projection"],
        },
        "risk.update_without_where" => RuleMetadata {
            title: "UPDATE without WHERE",
            description: "The SQL artifact contains an UPDATE statement without a WHERE clause.",
            impact: "A broad update can unintentionally modify every row in the target table.",
            suggested_fix: "Add a WHERE clause, batch limit, transaction guard, or explicit operator confirmation before execution.",
            confidence: 0.9,
            tags: &["risk", "sql", "write"],
        },
        "risk.delete_without_where" => RuleMetadata {
            title: "DELETE without WHERE",
            description: "The SQL artifact contains a DELETE statement without a WHERE clause.",
            impact: "A broad delete can unintentionally remove every row in the target table.",
            suggested_fix: "Add a WHERE clause, batch limit, transaction guard, or explicit operator confirmation before execution.",
            confidence: 0.9,
            tags: &["risk", "sql", "write"],
        },
        "quality.missing_primary_key" => RuleMetadata {
            title: "Missing primary key",
            description: "The table has no primary key constraint in the snapshot.",
            impact: "Rows may be hard to address reliably, and downstream sync or update logic can become ambiguous.",
            suggested_fix: "Add a primary key, or document the table as append-only, staging, or intentionally keyless.",
            confidence: 0.85,
            tags: &["quality", "constraint", "primary_key"],
        },
        "quality.probable_missing_fk" => RuleMetadata {
            title: "Probable missing foreign key",
            description: "A column name looks like a foreign key but no FK constraint or reference edge was found.",
            impact: "Relationship integrity may rely on application code and can drift over time.",
            suggested_fix: "Add an explicit foreign key if the relationship is required, or document the denormalized design choice.",
            confidence: 0.7,
            tags: &["quality", "constraint", "foreign_key"],
        },
        "performance.filter_without_index" => RuleMetadata {
            title: "Filter without supporting index",
            description: "SQL workload filters by this column but no supporting index was found in the snapshot.",
            impact: "Frequent filters can degrade into table scans as data volume grows.",
            suggested_fix: "Validate selectivity with EXPLAIN, then consider CREATE INDEX CONCURRENTLY on the filtered column for Postgres.",
            confidence: 0.75,
            tags: &["performance", "index", "filter"],
        },
        "performance.join_without_index" => RuleMetadata {
            title: "Join without supporting index",
            description: "SQL workload joins on this column but no supporting index was found in the snapshot.",
            impact: "Join-heavy paths can become slower as related tables grow.",
            suggested_fix: "Validate join cardinality with EXPLAIN, then consider CREATE INDEX CONCURRENTLY on the join column for Postgres.",
            confidence: 0.75,
            tags: &["performance", "index", "join"],
        },
        _ => RuleMetadata {
            title: "Analysis finding",
            description: "DbGraph detected a database graph condition that needs review.",
            impact: "The finding may affect data safety, quality, or performance.",
            suggested_fix: "Review the object, evidence, and related schema or SQL artifacts before changing production systems.",
            confidence: 0.5,
            tags: &["analysis"],
        },
    }
}

fn data_rule_metadata(rule_id: &str) -> Option<RuleMetadata> {
    Some(match rule_id {
        "data.enum_like_without_constraint" => RuleMetadata {
            title: "Enum-like sample without constraint",
            description: "Allowlisted samples show a small repeated value set, but the schema has no explicit constraint.",
            impact: "Application code may accept unexpected states or drift from the intended business workflow.",
            suggested_fix: "Add a CHECK constraint, enum type, or reference table if the value set is intentional.",
            confidence: 0.65,
            tags: &["quality", "data_profile", "business_rule"],
        },
        "data.high_null_rate" => RuleMetadata {
            title: "High null rate in sample",
            description: "Allowlisted samples show many nulls on a column that appears important to business state.",
            impact: "Reports, filters, or workflow logic may need to handle missing values explicitly.",
            suggested_fix: "Confirm whether null is valid, then add a NOT NULL/default rule or document the lifecycle state.",
            confidence: 0.6,
            tags: &["quality", "data_profile", "nullability"],
        },
        "data.negative_numeric_sample" => RuleMetadata {
            title: "Negative numeric sample",
            description: "Allowlisted samples include negative values for a metric-like column.",
            impact: "Unexpected negative values can break billing, inventory, or reporting assumptions.",
            suggested_fix: "Confirm whether negative values are valid, then add a CHECK constraint or document the business exception.",
            confidence: 0.65,
            tags: &["quality", "data_profile", "numeric"],
        },
        "data.unstable_format_sample" => RuleMetadata {
            title: "Unstable sample format",
            description: "Allowlisted samples show multiple coarse formats for an identifier-like column.",
            impact: "Inconsistent formats can make joins, validation, and downstream parsing unreliable.",
            suggested_fix: "Normalize existing values, add input validation, or document all supported formats explicitly.",
            confidence: 0.55,
            tags: &["quality", "data_profile", "format"],
        },
        _ => return None,
    })
}
