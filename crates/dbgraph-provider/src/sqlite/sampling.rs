//! Allowlisted `SQLite` row sampling.

use std::fmt::Write as _;

use dbgraph_core::config::{DataAccessConfig, DataAccessMode, DataAccessTableRule};
use dbgraph_core::model::DbSnapshot;
use dbgraph_core::sampling::{summarize_column_values, SamplingOptions};
use dbgraph_core::security::{PiiDetector, PiiRuleConfig};
use dbgraph_core::{DbGraphError, Result};
use rusqlite::Connection;
use serde_json::{json, Value};

use crate::sampling_support::upsert_sample_summary;

use super::{quote_identifier, sqlite_error};

pub(super) fn apply_sqlite_samples(
    connection: &Connection,
    snapshot: &mut DbSnapshot,
    data_access: &DataAccessConfig,
    sampling: &SamplingOptions,
) -> Result<()> {
    let detector = PiiDetector::new(&PiiRuleConfig::default());
    let tables = snapshot
        .objects
        .iter()
        .filter(|object| object.kind.as_str() == "table")
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
        let table_name = table.table_name.as_deref().unwrap_or(&table.name);
        let query = build_sqlite_sample_query(table_name, rule, sampling.max_rows_per_table)?;
        let mut statement = connection.prepare(&query).map_err(sqlite_error)?;
        let mut values_by_column = vec![Vec::new(); rule.columns.len()];
        let mut rows = statement.query([]).map_err(sqlite_error)?;
        while let Some(row) = rows.next().map_err(sqlite_error)? {
            for (index, values) in values_by_column.iter_mut().enumerate() {
                values.push(sqlite_value_to_json(
                    row.get_ref(index).map_err(sqlite_error)?,
                ));
            }
        }
        for (column_name, values) in rule.columns.iter().zip(values_by_column) {
            let Some(column) = snapshot
                .objects
                .iter()
                .find(|object| {
                    object.kind.as_str() == "column"
                        && object.table_name.as_deref() == Some(table_name)
                        && object.column_name.as_deref() == Some(column_name.as_str())
                })
                .cloned()
            else {
                continue;
            };
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

fn build_sqlite_sample_query(
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
        .map(|column| quote_identifier(column))
        .collect::<Vec<_>>()
        .join(", ");
    let mut query = format!("SELECT {columns} FROM {}", quote_identifier(table));
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

fn sqlite_value_to_json(value: rusqlite::types::ValueRef<'_>) -> Value {
    match value {
        rusqlite::types::ValueRef::Null => Value::Null,
        rusqlite::types::ValueRef::Integer(value) => Value::Number(value.into()),
        rusqlite::types::ValueRef::Real(value) => {
            serde_json::Number::from_f64(value).map_or(Value::Null, Value::Number)
        }
        rusqlite::types::ValueRef::Text(value) => {
            Value::String(String::from_utf8_lossy(value).into_owned())
        }
        rusqlite::types::ValueRef::Blob(value) => Value::String(format!("<{} bytes>", value.len())),
    }
}
