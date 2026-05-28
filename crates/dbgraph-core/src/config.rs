//! Configuration model and loader for `.dbgraph/dbgraph.config.json`.

use std::fmt;
use std::fs;
use std::path::Path;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::project::ProjectContext;
use crate::{DbGraphError, Result};

/// Supported database providers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DatabaseProviderKind {
    /// `PostgreSQL` provider.
    Postgres,
    /// `MySQL` provider.
    Mysql,
    /// SQL Server provider.
    SqlServer,
    /// `SQLite` provider.
    Sqlite,
}

impl DatabaseProviderKind {
    /// Returns all supported provider ids.
    #[must_use]
    pub const fn supported_values() -> &'static [&'static str] {
        &["postgres", "mysql", "sql-server", "sqlite"]
    }
}

impl fmt::Display for DatabaseProviderKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = match self {
            Self::Postgres => "postgres",
            Self::Mysql => "mysql",
            Self::SqlServer => "sql-server",
            Self::Sqlite => "sqlite",
        };
        f.write_str(value)
    }
}

impl FromStr for DatabaseProviderKind {
    type Err = String;

    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        match value {
            "postgres" => Ok(Self::Postgres),
            "mysql" => Ok(Self::Mysql),
            "sql-server" => Ok(Self::SqlServer),
            "sqlite" => Ok(Self::Sqlite),
            _ => Err(format!(
                "unsupported database.provider `{value}`; supported values: {}",
                DatabaseProviderKind::supported_values().join(", ")
            )),
        }
    }
}

/// Database connection configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseConfig {
    /// Selected database provider.
    pub provider: String,
    /// Environment variable containing the connection string.
    pub connection_env: Option<String>,
    /// Literal connection string, supported only as a lower-priority fallback.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connection_string: Option<String>,
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            provider: DatabaseProviderKind::Postgres.to_string(),
            connection_env: Some("DATABASE_URL".to_owned()),
            connection_string: None,
        }
    }
}

impl DatabaseConfig {
    /// Parses the configured provider into a supported enum value.
    ///
    /// # Errors
    ///
    /// Returns an invalid configuration error when the provider is unknown.
    pub fn provider_kind(&self) -> Result<DatabaseProviderKind> {
        self.provider
            .parse::<DatabaseProviderKind>()
            .map_err(DbGraphError::invalid_config)
    }
}

/// Snapshot behavior configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotConfig {
    /// Whether to use pretty JSON when writing snapshots.
    pub pretty_json: bool,
    /// Whether to collect row samples. This must stay opt-in.
    pub sample_rows: bool,
}

impl Default for SnapshotConfig {
    fn default() -> Self {
        Self {
            pretty_json: true,
            sample_rows: false,
        }
    }
}

/// Security defaults for profiling and sample storage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SecurityConfig {
    /// Whether raw business row data may be stored. Defaults to false.
    pub store_raw_data: bool,
    /// Whether PII-like sample values should be masked.
    pub mask_pii: bool,
}

impl Default for SecurityConfig {
    fn default() -> Self {
        Self {
            store_raw_data: false,
            mask_pii: true,
        }
    }
}

/// MCP server configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpConfig {
    /// Whether MCP serving is enabled for this project.
    pub enabled: bool,
    /// Maximum response size budget in characters.
    pub max_response_chars: u32,
}

impl Default for McpConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_response_chars: 15_000,
        }
    }
}

/// Root configuration model for `.dbgraph/dbgraph.config.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DbGraphConfig {
    /// Config schema version.
    pub version: u32,
    /// Database settings.
    pub database: DatabaseConfig,
    /// Snapshot settings.
    pub snapshot: SnapshotConfig,
    /// Security settings.
    pub security: SecurityConfig,
    /// MCP settings.
    pub mcp: McpConfig,
}

impl Default for DbGraphConfig {
    fn default() -> Self {
        Self {
            version: 1,
            database: DatabaseConfig::default(),
            snapshot: SnapshotConfig::default(),
            security: SecurityConfig::default(),
            mcp: McpConfig::default(),
        }
    }
}

impl DbGraphConfig {
    /// Loads and validates config from a project context.
    ///
    /// # Errors
    ///
    /// Returns a config error when the config file is missing or invalid, and
    /// an I/O error when the file cannot be read.
    pub fn load(context: &ProjectContext) -> Result<Self> {
        Self::load_from_path(context.config_path())
    }

    /// Loads and validates config from a path.
    ///
    /// # Errors
    ///
    /// Returns a config error when the config file is missing or malformed, and
    /// an I/O error when the file cannot be read.
    pub fn load_from_path(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if !path.exists() {
            return Err(DbGraphError::ConfigNotFound {
                path: path.to_path_buf(),
            });
        }

        let content = fs::read_to_string(path).map_err(|source| DbGraphError::io(path, source))?;
        let config = serde_json::from_str::<Self>(&content).map_err(|source| {
            DbGraphError::invalid_config(format!(
                "failed to parse {}: {source}. Re-run `dbgraph init --force` to regenerate a valid config.",
                path.display()
            ))
        })?;

        config.validate()?;
        Ok(config)
    }

    /// Saves config to a project context.
    ///
    /// # Errors
    ///
    /// Returns an error if validation fails or the file cannot be written.
    pub fn save(&self, context: &ProjectContext) -> Result<()> {
        self.save_to_path(context.config_path())
    }

    /// Saves config to a path as pretty JSON.
    ///
    /// # Errors
    ///
    /// Returns an error if validation fails, serialization fails, or the file
    /// cannot be written.
    pub fn save_to_path(&self, path: impl AsRef<Path>) -> Result<()> {
        self.validate()?;
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|source| DbGraphError::io(parent, source))?;
        }

        let content =
            serde_json::to_string_pretty(self).map_err(|source| DbGraphError::Internal {
                message: format!("failed to serialize config: {source}"),
            })?;
        fs::write(path, format!("{content}\n")).map_err(|source| DbGraphError::io(path, source))
    }

    /// Validates the config and returns user-actionable errors.
    ///
    /// # Errors
    ///
    /// Returns an invalid configuration error when required values are empty or
    /// unsafe defaults are requested.
    pub fn validate(&self) -> Result<()> {
        if self.version == 0 {
            return Err(DbGraphError::invalid_config(
                "version must be greater than zero",
            ));
        }

        self.database.provider_kind()?;

        if self
            .database
            .connection_env
            .as_deref()
            .is_some_and(str::is_empty)
        {
            return Err(DbGraphError::invalid_config(
                "database.connectionEnv must not be empty",
            ));
        }

        if self.database.connection_env.is_none() && self.database.connection_string.is_some() {
            return Err(DbGraphError::invalid_config(
                "database.connectionEnv is required when database.connectionString is set; prefer environment variables over plaintext connection strings",
            ));
        }

        if self.snapshot.sample_rows && self.security.store_raw_data {
            return Err(DbGraphError::invalid_config(
                "snapshot.sampleRows and security.storeRawData cannot both be true; raw business row data is not stored by default",
            ));
        }

        if self.mcp.max_response_chars == 0 {
            return Err(DbGraphError::invalid_config(
                "mcp.maxResponseChars must be greater than zero",
            ));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn default_config_uses_safe_data_settings() {
        let config = DbGraphConfig::default();

        assert_eq!(
            config.database.provider_kind().ok(),
            Some(DatabaseProviderKind::Postgres)
        );
        assert_eq!(
            config.database.connection_env.as_deref(),
            Some("DATABASE_URL")
        );
        assert_eq!(config.database.connection_string, None);
        assert!(!config.snapshot.sample_rows);
        assert!(!config.security.store_raw_data);
        assert!(config.security.mask_pii);
    }

    #[test]
    fn missing_config_suggests_init() {
        let temp = TempProject::new();
        let err = DbGraphConfig::load_from_path(temp.root.join("missing.json"))
            .expect_err("missing config should fail");

        assert!(err.to_string().contains("Run `dbgraph init` first"));
    }

    #[test]
    fn invalid_provider_returns_clear_error() {
        let temp = TempProject::new();
        let config_path = temp.root.join("dbgraph.config.json");
        fs::write(
            &config_path,
            r#"{
              "version": 1,
              "database": { "provider": "oracle", "connectionEnv": "DATABASE_URL" },
              "snapshot": { "prettyJson": true, "sampleRows": false },
              "security": { "storeRawData": false, "maskPii": true },
              "mcp": { "enabled": true, "maxResponseChars": 15000 }
            }"#,
        )
        .expect("config should be written");

        let err = DbGraphConfig::load_from_path(&config_path).expect_err("provider should fail");

        assert!(err.to_string().contains("unsupported database.provider"));
        assert!(err.to_string().contains("postgres"));
    }

    #[test]
    fn save_and_load_round_trip() {
        let temp = TempProject::new();
        let context = ProjectContext::from_project_root(&temp.root);
        let config = DbGraphConfig::default();

        config.save(&context).expect("config should save");
        let loaded = DbGraphConfig::load(&context).expect("config should load");

        assert_eq!(loaded, config);
        assert!(context.config_path().exists());
    }

    #[test]
    fn rejects_plaintext_connection_without_env_reference() {
        let config = DbGraphConfig {
            database: DatabaseConfig {
                provider: "postgres".to_owned(),
                connection_env: None,
                connection_string: Some("postgres://user:pass@localhost/db".to_owned()),
            },
            ..DbGraphConfig::default()
        };

        let err = config
            .validate()
            .expect_err("plaintext-only config should fail");

        assert!(err.to_string().contains("connectionEnv is required"));
    }

    #[test]
    fn rejects_raw_storage_with_sampling_enabled() {
        let config = DbGraphConfig {
            snapshot: SnapshotConfig {
                sample_rows: true,
                ..SnapshotConfig::default()
            },
            security: SecurityConfig {
                store_raw_data: true,
                ..SecurityConfig::default()
            },
            ..DbGraphConfig::default()
        };

        let err = config
            .validate()
            .expect_err("unsafe data settings should fail");

        assert!(err.to_string().contains("storeRawData"));
    }

    struct TempProject {
        root: PathBuf,
    }

    impl TempProject {
        fn new() -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock should be valid")
                .as_nanos();
            let root = std::env::temp_dir().join(format!(
                "dbgraph-config-test-{}-{unique}",
                std::process::id()
            ));
            fs::create_dir_all(&root).expect("temp root should be created");
            Self { root }
        }
    }

    impl Drop for TempProject {
        fn drop(&mut self) {
            if self.root.exists() {
                fs::remove_dir_all(&self.root).expect("temp root should be removed");
            }
        }
    }
}
