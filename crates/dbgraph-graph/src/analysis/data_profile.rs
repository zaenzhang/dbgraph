//! Sample-derived analysis findings.

use dbgraph_core::model::{DbObject, DbObjectKind, DbSnapshot};

use super::{finding, object_by_id, AnalysisFinding, AnalysisScope, FindingSeverity};

pub(super) fn data_profile_findings(snapshot: &DbSnapshot, findings: &mut Vec<AnalysisFinding>) {
    for profile in &snapshot.column_profiles {
        let Some(summary) = profile.profile.get("sampleSummary") else {
            continue;
        };
        let Some(object) = object_by_id(snapshot, &profile.object_id) else {
            continue;
        };
        let observed_non_null = summary
            .get("observedNonNull")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or_default();
        let observed_null = summary
            .get("observedNull")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or_default();
        let distinct_count = summary
            .get("distinctCount")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or_default();
        let inferred_shape = summary
            .get("inferredShape")
            .and_then(serde_json::Value::as_str);
        let numeric_min = summary
            .get("numericMin")
            .and_then(serde_json::Value::as_f64);
        let format_shape_count = summary
            .get("formatShapes")
            .and_then(serde_json::Value::as_array)
            .map_or(0, Vec::len);

        if inferred_shape == Some("enum_like")
            && distinct_count > 1
            && !column_has_check_or_enum_constraint(snapshot, object)
        {
            findings.push(finding(
                AnalysisScope::Quality,
                FindingSeverity::Low,
                "data.enum_like_without_constraint",
                object,
                "Sampled values look enum-like but no explicit constraint was found",
                format!("distinctCount={distinct_count}; source=sample"),
                Vec::new(),
            ));
        }

        let total = observed_non_null + observed_null;
        if total >= 5 && observed_null * 2 > total && important_column_name(object) {
            findings.push(finding(
                AnalysisScope::Quality,
                FindingSeverity::Medium,
                "data.high_null_rate",
                object,
                "Allowlisted sample shows a high null rate on an important-looking column",
                format!("observedNull={observed_null}; observedNonNull={observed_non_null}"),
                Vec::new(),
            ));
        }

        if inferred_shape == Some("numeric")
            && numeric_min.is_some_and(|minimum| minimum < 0.0)
            && non_negative_business_column(object)
        {
            findings.push(finding(
                AnalysisScope::Quality,
                FindingSeverity::Medium,
                "data.negative_numeric_sample",
                object,
                "Allowlisted sample contains negative values on a column that usually should not be negative",
                format!("numericMin={}", numeric_min.unwrap_or_default()),
                Vec::new(),
            ));
        }

        if observed_non_null >= 5 && format_shape_count > 1 && format_sensitive_column_name(object)
        {
            findings.push(finding(
                AnalysisScope::Quality,
                FindingSeverity::Low,
                "data.unstable_format_sample",
                object,
                "Allowlisted sample shows multiple observed formats for an identifier-like column",
                format!("formatShapeCount={format_shape_count}; source=sample"),
                Vec::new(),
            ));
        }
    }
}

fn column_has_check_or_enum_constraint(snapshot: &DbSnapshot, column: &DbObject) -> bool {
    let Some(table_name) = column.table_name.as_deref() else {
        return false;
    };
    let Some(column_name) = column.column_name.as_deref() else {
        return false;
    };
    snapshot.objects.iter().any(|object| {
        matches!(
            object.kind,
            DbObjectKind::CheckConstraint | DbObjectKind::Enum
        ) && (object.table_name.as_deref() == Some(table_name)
            || object.full_name.contains(table_name))
            && object.constraint.as_ref().map_or(true, |constraint| {
                constraint.columns.iter().any(|name| name == column_name)
            })
    })
}

fn important_column_name(column: &DbObject) -> bool {
    column
        .column_name
        .as_deref()
        .is_some_and(|name| matches!(name, "status" | "state" | "type" | "kind" | "email"))
}

fn non_negative_business_column(column: &DbObject) -> bool {
    column.column_name.as_deref().is_some_and(|name| {
        let name = name.to_ascii_lowercase();
        ["amount", "price", "quantity", "total", "balance", "count"]
            .iter()
            .any(|term| name.contains(term))
    })
}

fn format_sensitive_column_name(column: &DbObject) -> bool {
    column.column_name.as_deref().is_some_and(|name| {
        let name = name.to_ascii_lowercase();
        name.ends_with("_id")
            || ["code", "sku", "email", "url", "uuid", "slug"]
                .iter()
                .any(|term| name.contains(term))
    })
}
