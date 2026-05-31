//! Status and doctor CLI command implementations.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use dbgraph_core::config::DbGraphConfig;
use dbgraph_core::model::DbObjectKind;
use dbgraph_core::project::ProjectContext;
use dbgraph_core::{DbGraphError, Result};
use dbgraph_provider::{ProviderConnectionConfig, ProviderRegistry};
use serde::Serialize;

use crate::commands::common::latest_snapshot;
use crate::commands::snapshot::resolve_connection_url;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StatusReport {
    pub(crate) project_root: PathBuf,
    pub(crate) dbgraph_dir: PathBuf,
    pub(crate) initialized: bool,
    pub(crate) config_path: PathBuf,
    pub(crate) config_present: bool,
    pub(crate) provider: Option<String>,
    pub(crate) snapshot_count: usize,
    pub(crate) latest_snapshot: Option<String>,
    pub(crate) graph_db_path: PathBuf,
    pub(crate) graph_db_present: bool,
    pub(crate) mcp_suggestion: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DoctorStatus {
    Ok,
    Warning,
    Error,
}

impl DoctorStatus {
    const fn rank(self) -> u8 {
        match self {
            Self::Ok => 0,
            Self::Warning => 1,
            Self::Error => 2,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DoctorCheck {
    pub(crate) id: String,
    pub(crate) status: DoctorStatus,
    pub(crate) message: String,
    pub(crate) suggestion: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DoctorReport {
    pub(crate) status: DoctorStatus,
    pub(crate) project_root: PathBuf,
    pub(crate) checks: Vec<DoctorCheck>,
    pub(crate) suggested_next_commands: Vec<String>,
}

pub(crate) fn read_status(start: impl AsRef<Path>) -> Result<StatusReport> {
    let start = start.as_ref();
    let context = match ProjectContext::discover_from(start)? {
        Some(context) => context,
        None => ProjectContext::from_project_root(start),
    };
    let config_path = context.config_path();
    let config_present = config_path.is_file();
    let provider = if config_present {
        Some(DbGraphConfig::load(&context)?.database.provider)
    } else {
        None
    };
    let snapshots = snapshot_files(&context)?;
    let latest_snapshot = snapshots
        .last()
        .and_then(|path| path.file_name())
        .map(|name| name.to_string_lossy().into_owned());

    Ok(StatusReport {
        project_root: context.project_root().to_path_buf(),
        dbgraph_dir: context.dbgraph_dir().to_path_buf(),
        initialized: context.dbgraph_dir().is_dir() && config_present,
        config_path,
        config_present,
        provider,
        snapshot_count: snapshots.len(),
        latest_snapshot,
        graph_db_path: context.graph_db_path(),
        graph_db_present: context.graph_db_path().is_file(),
        mcp_suggestion: "Run `dbgraph serve --mcp` to start the MCP stdio server.".to_owned(),
    })
}

fn snapshot_files(context: &ProjectContext) -> Result<Vec<PathBuf>> {
    if !context.snapshots_dir().is_dir() {
        return Ok(Vec::new());
    }

    let mut files = fs::read_dir(context.snapshots_dir())
        .map_err(|source| DbGraphError::io(context.snapshots_dir(), source))?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect::<Vec<_>>();
    files.sort_by_key(|path| {
        fs::metadata(path)
            .and_then(|metadata| metadata.modified())
            .ok()
            .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |duration| duration.as_secs())
    });
    Ok(files)
}

pub(crate) fn print_status(status: &StatusReport) {
    if !status.initialized {
        println!("DbGraph is not initialized.");
        println!("Run `dbgraph init` in this project.");
        println!("Checked: {}", status.dbgraph_dir.display());
        return;
    }

    println!("DbGraph status");
    println!("Project: {}", status.project_root.display());
    println!(".dbgraph: present");
    println!("Config: {}", status.config_path.display());
    println!(
        "Provider: {}",
        status.provider.as_deref().unwrap_or("unknown")
    );
    println!("Snapshots: {}", status.snapshot_count);
    println!(
        "Latest snapshot: {}",
        status.latest_snapshot.as_deref().unwrap_or("none")
    );
    println!(
        "Graph index: {}",
        if status.graph_db_present {
            status.graph_db_path.display().to_string()
        } else {
            "missing".to_owned()
        }
    );
    println!("MCP: {}", status.mcp_suggestion);
}

#[allow(clippy::too_many_lines)]
pub(crate) fn doctor_project(start: impl AsRef<Path>, check_db: bool) -> Result<DoctorReport> {
    let start = start.as_ref();
    let context = ProjectContext::discover_from(start)?
        .unwrap_or_else(|| ProjectContext::from_project_root(start));
    let mut checks = Vec::new();
    push_check(
        &mut checks,
        "project",
        DoctorStatus::Ok,
        format!("Project root: {}", context.project_root().display()),
        None::<String>,
    );
    push_check(
        &mut checks,
        "dbgraph_dir",
        if context.dbgraph_dir().is_dir() {
            DoctorStatus::Ok
        } else {
            DoctorStatus::Error
        },
        if context.dbgraph_dir().is_dir() {
            format!("Found {}", context.dbgraph_dir().display())
        } else {
            format!("Missing {}", context.dbgraph_dir().display())
        },
        (!context.dbgraph_dir().is_dir()).then_some("dbgraph init -i --yes".to_owned()),
    );

    let config = match DbGraphConfig::load(&context) {
        Ok(config) => {
            push_check(
                &mut checks,
                "config",
                DoctorStatus::Ok,
                format!("Config is valid: {}", context.config_path().display()),
                None::<String>,
            );
            Some(config)
        }
        Err(err) => {
            push_check(
                &mut checks,
                "config",
                DoctorStatus::Error,
                err.to_string(),
                Some("dbgraph init -i --yes".to_owned()),
            );
            None
        }
    };

    if let Some(config) = &config {
        let provider_known = ProviderRegistry.get(&config.database.provider).is_some();
        push_check(
            &mut checks,
            "provider",
            if provider_known {
                DoctorStatus::Ok
            } else {
                DoctorStatus::Error
            },
            format!("Provider: {}", config.database.provider),
            (!provider_known).then_some("choose a supported database.provider".to_owned()),
        );
        if let Some(env_name) = config.database.connection_env.as_deref() {
            let present = env::var(env_name).is_ok_and(|value| !value.trim().is_empty());
            push_check(
                &mut checks,
                "connection_env",
                if present {
                    DoctorStatus::Ok
                } else if config.database.connection_string.is_some() {
                    DoctorStatus::Warning
                } else {
                    DoctorStatus::Error
                },
                if present {
                    format!("Environment variable {env_name} is set")
                } else {
                    format!("Environment variable {env_name} is not set")
                },
                (!present && config.database.connection_string.is_none())
                    .then_some(format!("set {env_name} or database.connectionString")),
            );
        }
        push_check(
            &mut checks,
            "mcp",
            if config.mcp.enabled {
                DoctorStatus::Ok
            } else {
                DoctorStatus::Warning
            },
            if config.mcp.enabled {
                format!(
                    "MCP enabled with maxResponseChars={}",
                    config.mcp.max_response_chars
                )
            } else {
                "MCP disabled in config".to_owned()
            },
            (!config.mcp.enabled).then_some("enable mcp.enabled in dbgraph.config.json".to_owned()),
        );
        if check_db {
            checks.push(check_database_connection(config));
        }
    }

    let snapshots = snapshot_files(&context)?;
    push_check(
        &mut checks,
        "snapshots",
        if snapshots.is_empty() {
            DoctorStatus::Warning
        } else {
            DoctorStatus::Ok
        },
        if snapshots.is_empty() {
            "No snapshots found".to_owned()
        } else {
            format!("{} snapshot(s) found", snapshots.len())
        },
        snapshots
            .is_empty()
            .then_some("dbgraph snapshot --profile stats".to_owned()),
    );
    push_check(
        &mut checks,
        "graph_index",
        if context.graph_db_path().is_file() {
            DoctorStatus::Ok
        } else {
            DoctorStatus::Warning
        },
        if context.graph_db_path().is_file() {
            format!("Graph index exists: {}", context.graph_db_path().display())
        } else {
            "Graph index is missing".to_owned()
        },
        (!context.graph_db_path().is_file())
            .then_some("dbgraph snapshot --profile stats".to_owned()),
    );
    let sql_artifact_count = latest_snapshot(&context).ok().map_or(0, |snapshot| {
        snapshot
            .objects
            .iter()
            .filter(|object| object.kind == DbObjectKind::Query)
            .count()
    });
    push_check(
        &mut checks,
        "sql_artifacts",
        DoctorStatus::Ok,
        format!("{sql_artifact_count} SQL artifact(s) in latest snapshot"),
        None::<String>,
    );
    let path_visible = command_visible_on_path("dbgraph");
    push_check(
        &mut checks,
        "path",
        if path_visible {
            DoctorStatus::Ok
        } else {
            DoctorStatus::Warning
        },
        if path_visible {
            "`dbgraph` is visible on PATH".to_owned()
        } else {
            "`dbgraph` was not found on PATH".to_owned()
        },
        (!path_visible).then_some("install dbgraph or add it to PATH".to_owned()),
    );

    let status = checks
        .iter()
        .map(|check| check.status)
        .max_by_key(|status| status.rank())
        .unwrap_or(DoctorStatus::Ok);
    let mut suggested_next_commands = checks
        .iter()
        .filter_map(|check| check.suggestion.clone())
        .filter(|suggestion| suggestion.starts_with("dbgraph "))
        .collect::<Vec<_>>();
    suggested_next_commands.sort();
    suggested_next_commands.dedup();
    if context.config_path().is_file()
        && !suggested_next_commands.contains(&"dbgraph install --target codex --yes".to_owned())
    {
        suggested_next_commands.push("dbgraph install --target codex --yes".to_owned());
    }
    Ok(DoctorReport {
        status,
        project_root: context.project_root().to_path_buf(),
        checks,
        suggested_next_commands,
    })
}

fn push_check(
    checks: &mut Vec<DoctorCheck>,
    id: &str,
    status: DoctorStatus,
    message: String,
    suggestion: Option<String>,
) {
    checks.push(DoctorCheck {
        id: id.to_owned(),
        status,
        message,
        suggestion,
    });
}

fn check_database_connection(config: &DbGraphConfig) -> DoctorCheck {
    let Some(provider) = ProviderRegistry.get(&config.database.provider) else {
        return DoctorCheck {
            id: "database_connection".to_owned(),
            status: DoctorStatus::Error,
            message: format!("Unknown provider {}", config.database.provider),
            suggestion: Some("choose a supported database.provider".to_owned()),
        };
    };
    match resolve_connection_url(&config.database)
        .map(ProviderConnectionConfig::from_url)
        .and_then(|connection| provider.connect(&connection))
    {
        Ok(info) => DoctorCheck {
            id: "database_connection".to_owned(),
            status: DoctorStatus::Ok,
            message: format!(
                "Connected to {} as {}",
                info.database_name, info.current_user
            ),
            suggestion: None,
        },
        Err(err) => DoctorCheck {
            id: "database_connection".to_owned(),
            status: DoctorStatus::Error,
            message: err.to_string(),
            suggestion: Some("verify database connection settings".to_owned()),
        },
    }
}

fn command_visible_on_path(command: &str) -> bool {
    let Some(paths) = env::var_os("PATH") else {
        return false;
    };
    env::split_paths(&paths).any(|path| {
        let candidate = path.join(command);
        candidate.is_file() || candidate.with_extension("exe").is_file()
    })
}

pub(crate) fn print_doctor_report(report: &DoctorReport) {
    println!("DbGraph doctor: {:?}", report.status);
    println!("Project: {}", report.project_root.display());
    for check in &report.checks {
        println!("- {:?} {}: {}", check.status, check.id, check.message);
        if let Some(suggestion) = &check.suggestion {
            println!("  suggestion: {suggestion}");
        }
    }
    if !report.suggested_next_commands.is_empty() {
        println!("Suggested next commands:");
        for command in &report.suggested_next_commands {
            println!("- {command}");
        }
    }
}
