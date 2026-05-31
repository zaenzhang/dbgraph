//! Safe sample summary helpers.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;

use crate::model::DbObject;
use crate::security::{mask_value, MaskingStrategy, PiiDetector};

/// Sample extraction strategy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SamplingStrategy {
    /// Use deterministic limit sampling.
    Limit,
    /// Use database random sampling when a provider supports it.
    Random,
}

/// Safe sampler options.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SamplingOptions {
    /// Maximum rows per table.
    pub max_rows_per_table: u32,
    /// Sampling strategy.
    pub strategy: SamplingStrategy,
    /// Optional statement timeout in milliseconds.
    pub statement_timeout_ms: Option<u64>,
    /// Whether raw non-sensitive values may be retained.
    pub store_raw_samples: bool,
    /// Masking strategy for sensitive values.
    pub masking_strategy: MaskingStrategy,
}

/// Per-column sample summary that avoids sensitive raw values by default.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ColumnSampleSummary {
    /// Column full name.
    pub column: String,
    /// Number of observed non-null values.
    pub observed_non_null: usize,
    /// Number of observed null values.
    pub observed_null: usize,
    /// Whether this column was considered sensitive.
    pub sensitive: bool,
    /// Masked or raw examples according to policy.
    pub examples: Vec<Value>,
    /// Number of distinct non-null values observed in the bounded sample.
    pub distinct_count: usize,
    /// Basic inferred shape of the sample, such as `enum_like` or `numeric`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inferred_shape: Option<String>,
    /// Minimum numeric value observed when the sample is numeric-like.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub numeric_min: Option<f64>,
    /// Maximum numeric value observed when the sample is numeric-like.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub numeric_max: Option<f64>,
    /// Distinct coarse string formats observed in the sample.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub format_shapes: Vec<String>,
    /// Summary source.
    pub source: String,
}

/// Summarizes already-fetched row values with masking policy.
#[must_use]
pub fn summarize_column_values(
    column: &DbObject,
    values: &[Value],
    detector: &PiiDetector,
    options: &SamplingOptions,
) -> ColumnSampleSummary {
    let finding = detector.detect_column(column);
    let sensitive = finding.score >= 0.4;
    let mut observed_non_null = 0;
    let mut observed_null = 0;
    let mut examples = Vec::new();
    let mut distinct = BTreeSet::new();
    let mut example_keys = BTreeSet::new();
    let mut all_numeric = true;
    let mut all_text = true;
    let mut numeric_min: Option<f64> = None;
    let mut numeric_max: Option<f64> = None;
    let mut format_shapes = BTreeSet::new();

    for value in values.iter().take(options.max_rows_per_table as usize) {
        if value.is_null() {
            observed_null += 1;
            continue;
        }
        observed_non_null += 1;
        let numeric = numeric_value(value);
        if let Some(number) = numeric {
            numeric_min = Some(numeric_min.map_or(number, |current| current.min(number)));
            numeric_max = Some(numeric_max.map_or(number, |current| current.max(number)));
        }
        all_numeric &= numeric.is_some();
        all_text &= value.as_str().is_some();
        distinct.insert(value_to_string(value));
        if let Some(text) = value.as_str() {
            format_shapes.insert(format_shape(text));
        }
        if examples.len() >= 5 {
            continue;
        }
        let text = value_to_string(value);
        if !example_keys.insert(text.clone()) {
            continue;
        }
        let stored = if sensitive || !options.store_raw_samples {
            Value::String(mask_value(&text, options.masking_strategy))
        } else {
            value.clone()
        };
        examples.push(stored);
    }

    let inferred_shape = if temporal_column(column) {
        Some("timestamp".to_owned())
    } else {
        infer_shape(observed_non_null, distinct.len(), all_numeric, all_text)
    };

    ColumnSampleSummary {
        column: column.full_name.clone(),
        observed_non_null,
        observed_null,
        sensitive,
        examples,
        distinct_count: distinct.len(),
        inferred_shape,
        numeric_min,
        numeric_max,
        format_shapes: format_shapes.into_iter().collect(),
        source: "sample".to_owned(),
    }
}

fn infer_shape(
    observed_non_null: usize,
    distinct_count: usize,
    all_numeric: bool,
    all_text: bool,
) -> Option<String> {
    if observed_non_null == 0 {
        return None;
    }
    if all_numeric {
        return Some("numeric".to_owned());
    }
    if all_text && distinct_count <= 10 && distinct_count <= observed_non_null {
        return Some("enum_like".to_owned());
    }
    if all_text {
        return Some("text".to_owned());
    }
    Some("mixed".to_owned())
}

fn value_to_string(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

fn numeric_value(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| {
            value
                .as_i64()
                .and_then(|number| number.to_string().parse::<f64>().ok())
        })
        .or_else(|| {
            value
                .as_u64()
                .and_then(|number| number.to_string().parse::<f64>().ok())
        })
        .or_else(|| value.as_str().and_then(|text| text.parse::<f64>().ok()))
}

fn format_shape(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_digit() {
                '9'
            } else if ch.is_ascii_uppercase() {
                'A'
            } else if ch.is_ascii_lowercase() {
                'a'
            } else {
                ch
            }
        })
        .collect()
}

fn temporal_column(column: &DbObject) -> bool {
    column
        .column
        .as_ref()
        .and_then(|metadata| {
            metadata
                .data_type_family
                .as_deref()
                .or(metadata.data_type.as_deref())
        })
        .is_some_and(|data_type| {
            let data_type = data_type.to_ascii_lowercase();
            data_type.contains("timestamp") || data_type == "date" || data_type.contains("time")
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ColumnMetadata, DbObject, DbObjectKind};
    use crate::security::PiiRuleConfig;

    #[test]
    fn sensitive_samples_are_masked_even_when_raw_samples_are_enabled() {
        let detector = PiiDetector::new(&PiiRuleConfig::default());
        let mut column = DbObject::new(
            "column:users.email",
            DbObjectKind::Column,
            "public.users.email",
        );
        column.column_name = Some("email".to_owned());
        column.column = Some(ColumnMetadata {
            data_type: Some("text".to_owned()),
            data_type_family: Some("text".to_owned()),
            nullable: Some(false),
            default: None,
            comment: None,
        });

        let summary = summarize_column_values(
            &column,
            &[Value::String("a@example.test".to_owned())],
            &detector,
            &SamplingOptions {
                max_rows_per_table: 10,
                strategy: SamplingStrategy::Limit,
                statement_timeout_ms: Some(1_000),
                store_raw_samples: true,
                masking_strategy: MaskingStrategy::Redact,
            },
        );

        assert!(summary.sensitive);
        assert_eq!(
            summary.examples,
            vec![Value::String("[REDACTED]".to_owned())]
        );
    }

    #[test]
    fn non_sensitive_raw_samples_are_optional_and_summaries_are_rich() {
        let detector = PiiDetector::new(&PiiRuleConfig::default());
        let mut column = DbObject::new(
            "column:orders.status",
            DbObjectKind::Column,
            "public.orders.status",
        );
        column.column_name = Some("status".to_owned());
        column.column = Some(ColumnMetadata {
            data_type: Some("text".to_owned()),
            data_type_family: Some("text".to_owned()),
            nullable: Some(false),
            default: None,
            comment: None,
        });

        let summary = summarize_column_values(
            &column,
            &[
                Value::String("open".to_owned()),
                Value::String("paid".to_owned()),
                Value::String("paid".to_owned()),
                Value::Null,
            ],
            &detector,
            &SamplingOptions {
                max_rows_per_table: 10,
                strategy: SamplingStrategy::Limit,
                statement_timeout_ms: Some(1_000),
                store_raw_samples: true,
                masking_strategy: MaskingStrategy::Redact,
            },
        );

        assert_eq!(summary.observed_non_null, 3);
        assert_eq!(summary.observed_null, 1);
        assert_eq!(summary.distinct_count, 2);
        assert_eq!(
            summary.examples,
            vec![
                Value::String("open".to_owned()),
                Value::String("paid".to_owned())
            ]
        );
        assert_eq!(summary.inferred_shape.as_deref(), Some("enum_like"));
        assert_eq!(summary.format_shapes, vec!["aaaa".to_owned()]);

        let masked = summarize_column_values(
            &column,
            &[Value::String("open".to_owned())],
            &detector,
            &SamplingOptions {
                max_rows_per_table: 10,
                strategy: SamplingStrategy::Limit,
                statement_timeout_ms: Some(1_000),
                store_raw_samples: false,
                masking_strategy: MaskingStrategy::Redact,
            },
        );

        assert_eq!(
            masked.examples,
            vec![Value::String("[REDACTED]".to_owned())]
        );
    }
}
