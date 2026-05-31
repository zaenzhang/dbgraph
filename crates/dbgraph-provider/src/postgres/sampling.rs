//! Allowlisted `PostgreSQL` row sampling.

use std::fmt::Write as _;

use dbgraph_core::config::{DataAccessConfig, DataAccessMode, DataAccessTableRule};
use dbgraph_core::model::{DbObjectKind, DbSnapshot};
use dbgraph_core::sampling::{summarize_column_values, SamplingOptions};
use dbgraph_core::security::{PiiDetector, PiiRuleConfig};
use dbgraph_core::{DbGraphError, Result};
use postgres::Client;
use serde_json::{json, Value};

use crate::sampling_support::upsert_sample_summary;

use super::{pg_error, quote_identifier};

/// Builds a deterministic, allowlisted `PostgreSQL` sample query.
///
/// # Errors
///
/// Returns an error when no columns are configured.
pub fn build_sample_query(
    schema: &str,
    table: &str,
    rule: &DataAccessTableRule,
    max_rows_per_table: u32,
) -> Result<String> {
    if rule.columns.is_empty() {
        return Err(DbGraphError::invalid_config(
            "dataAccess sample rules require sample.columns",
        ));
    }
    let columns = rule
        .columns
        .iter()
        .map(|column| {
            let identifier = quote_identifier(column);
            format!("({identifier})::text AS {identifier}")
        })
        .collect::<Vec<_>>()
        .join(", ");
    let mut query = format!(
        "SELECT {columns} FROM {}.{}",
        quote_identifier(schema),
        quote_identifier(table)
    );
    if let Some(where_clause) = &rule.where_clause {
        query.push_str(" WHERE ");
        query.push_str(where_clause);
    }
    let limit = rule
        .limit
        .unwrap_or(max_rows_per_table)
        .min(max_rows_per_table);
    let _ = write!(query, " LIMIT {limit}");
    Ok(query)
}

pub(super) fn apply_postgres_samples(
    client: &mut Client,
    snapshot: &mut DbSnapshot,
    data_access: &DataAccessConfig,
    sampling: &SamplingOptions,
) -> Result<()> {
    let detector = PiiDetector::new(&PiiRuleConfig::default());
    let tables = snapshot
        .objects
        .iter()
        .filter(|object| object.kind == DbObjectKind::Table)
        .cloned()
        .collect::<Vec<_>>();
    let mut matched_rules = Vec::new();
    for table in tables {
        let Some(rule) = data_access.rule_for_table(&table.full_name) else {
            continue;
        };
        matched_rules.push(json!({
            "table": table.full_name,
            "pattern": rule.pattern,
            "mode": rule.mode,
        }));
        if rule.mode != DataAccessMode::Sample {
            continue;
        }
        let schema = table.schema_name.as_deref().unwrap_or("public");
        let table_name = table.table_name.as_deref().unwrap_or(&table.name);
        let query = build_sample_query(schema, table_name, rule, sampling.max_rows_per_table)?;
        let rows = client.query(&query, &[]).map_err(pg_error)?;
        for column_name in &rule.columns {
            let Some(column) = snapshot
                .objects
                .iter()
                .find(|object| {
                    object.kind == DbObjectKind::Column
                        && object.schema_name.as_deref() == Some(schema)
                        && object.table_name.as_deref() == Some(table_name)
                        && object.column_name.as_deref() == Some(column_name.as_str())
                })
                .cloned()
            else {
                continue;
            };
            let values = rows
                .iter()
                .map(|row| {
                    row.try_get::<_, Option<String>>(column_name.as_str())
                        .ok()
                        .flatten()
                })
                .map(|value| value.map_or(Value::Null, Value::String))
                .collect::<Vec<_>>();
            let mut options = sampling.clone();
            options.store_raw_samples = sampling.store_raw_samples || rule.store_raw_values;
            let summary = summarize_column_values(&column, &values, &detector, &options);
            upsert_sample_summary(snapshot, &column.id, &summary)?;
        }
    }
    if !data_access.tables.is_empty() {
        snapshot.metadata.insert(
            "dataAccess".to_owned(),
            json!({
                "defaultMode": data_access.default_mode,
                "rules": data_access.tables.len(),
                "matchedRules": matched_rules
            }),
        );
    }
    Ok(())
}
