//! Explicit allowlist policy for business-row access.

use serde::{Deserialize, Serialize};

use crate::profiling::ProfilingMode;
use crate::{DbGraphError, Result};

/// Per-table business-row access mode.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DataAccessMode {
    /// Schema metadata only. No business row reads.
    #[default]
    SchemaOnly,
    /// Catalog or count-style statistics only.
    Stats,
    /// Bounded allowlisted row sampling.
    Sample,
}

/// One table-level data access rule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DataAccessTableRule {
    /// Fully qualified table pattern. Supports exact names and `*` wildcards.
    pub pattern: String,
    /// Access mode for matching tables.
    #[serde(default)]
    pub mode: DataAccessMode,
    /// Allowlisted columns for sample mode.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub columns: Vec<String>,
    /// Optional read-only filter condition.
    #[serde(default, rename = "where", skip_serializing_if = "Option::is_none")]
    pub where_clause: Option<String>,
    /// Optional per-rule sample limit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    /// Whether raw non-sensitive values may be stored for this rule.
    #[serde(default)]
    pub store_raw_values: bool,
}

impl Default for DataAccessTableRule {
    fn default() -> Self {
        Self {
            pattern: String::new(),
            mode: DataAccessMode::SchemaOnly,
            columns: Vec::new(),
            where_clause: None,
            limit: None,
            store_raw_values: false,
        }
    }
}

/// Explicit business-row access policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DataAccessConfig {
    /// Default mode for tables without an explicit rule.
    #[serde(default)]
    pub default_mode: DataAccessMode,
    /// Table-specific access rules.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tables: Vec<DataAccessTableRule>,
}

impl Default for DataAccessConfig {
    fn default() -> Self {
        Self {
            default_mode: DataAccessMode::SchemaOnly,
            tables: Vec::new(),
        }
    }
}

impl DataAccessConfig {
    /// Returns the first rule matching a fully qualified table name.
    #[must_use]
    pub fn rule_for_table<'a>(&'a self, table_name: &str) -> Option<&'a DataAccessTableRule> {
        self.tables
            .iter()
            .find(|rule| wildcard_match(&rule.pattern, table_name))
    }

    /// Validates the policy against snapshot sampling settings.
    ///
    /// # Errors
    ///
    /// Returns an invalid configuration error when allowlisted sampling is unsafe.
    pub fn validate(&self, profiling_mode: ProfilingMode, max_rows_per_table: u32) -> Result<()> {
        for rule in &self.tables {
            if rule.pattern.trim().is_empty() {
                return Err(DbGraphError::invalid_config(
                    "dataAccess.tables[].pattern must not be empty",
                ));
            }
            if rule.mode == DataAccessMode::Sample {
                validate_sample_rule(rule, profiling_mode, max_rows_per_table)?;
            }
        }
        Ok(())
    }
}

fn validate_sample_rule(
    rule: &DataAccessTableRule,
    profiling_mode: ProfilingMode,
    max_rows_per_table: u32,
) -> Result<()> {
    if profiling_mode != ProfilingMode::Sample {
        return Err(DbGraphError::invalid_config(
            "dataAccess sample rules require snapshot.profilingMode to be sample",
        ));
    }
    if rule.columns.is_empty() {
        return Err(DbGraphError::invalid_config(
            "dataAccess sample rules require sample.columns",
        ));
    }
    let limit = rule.limit.unwrap_or(max_rows_per_table);
    if limit == 0 {
        return Err(DbGraphError::invalid_config(
            "dataAccess sample rule limit must be greater than zero",
        ));
    }
    if limit > max_rows_per_table {
        return Err(DbGraphError::invalid_config(
            "dataAccess sample rule limit cannot exceed snapshot.maxRowsPerTable",
        ));
    }
    if let Some(where_clause) = &rule.where_clause {
        validate_read_only_where(where_clause)?;
    }
    Ok(())
}

fn validate_read_only_where(where_clause: &str) -> Result<()> {
    let upper = where_clause.to_ascii_uppercase();
    let banned = [
        ";",
        " INSERT ",
        " UPDATE ",
        " DELETE ",
        " DROP ",
        " ALTER ",
        " CREATE ",
        " TRUNCATE ",
        " MERGE ",
        " CALL ",
        " GRANT ",
        " REVOKE ",
    ];
    let padded = format!(" {upper} ");
    if banned
        .iter()
        .any(|term| *term == ";" && upper.contains(';') || *term != ";" && padded.contains(term))
    {
        return Err(DbGraphError::invalid_config(
            "dataAccess sample rule where clause must be read-only and contain no semicolons or write keywords",
        ));
    }
    Ok(())
}

fn wildcard_match(pattern: &str, value: &str) -> bool {
    if pattern == "*" || pattern == value {
        return true;
    }
    let Some((prefix, suffix)) = pattern.split_once('*') else {
        return false;
    };
    value.starts_with(prefix) && value.ends_with(suffix)
}
