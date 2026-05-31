//! Project initialization command implementation.

use std::fs;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

use dbgraph_agent_config::render_all_instruction_fragments;
use dbgraph_core::config::{DatabaseConfig, DatabaseProviderKind, DbGraphConfig};
use dbgraph_core::project::ProjectContext;
use dbgraph_core::{DbGraphError, Result};

use crate::commands::snapshot::SnapshotSummary;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InitSummary {
    pub(crate) dbgraph_dir: PathBuf,
    pub(crate) config_path: PathBuf,
    pub(crate) instructions_dir: PathBuf,
}

#[derive(Debug, Clone)]
pub(crate) struct InitRunSummary {
    pub(crate) init: InitSummary,
    pub(crate) snapshot: Option<SnapshotSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct InitOptions {
    pub(crate) config: DbGraphConfig,
    pub(crate) configure_agent: bool,
    pub(crate) run_snapshot: bool,
}

impl InitOptions {
    pub(crate) fn interactive_defaults() -> Self {
        Self {
            configure_agent: true,
            run_snapshot: false,
            ..Self::default()
        }
    }
}

pub(crate) fn init_project(
    project_root: impl AsRef<Path>,
    force: bool,
    options: &InitOptions,
) -> Result<InitSummary> {
    let project_root = project_root.as_ref();
    fs::create_dir_all(project_root).map_err(|source| DbGraphError::io(project_root, source))?;

    let context = ProjectContext::from_project_root(project_root);
    fs::create_dir_all(context.dbgraph_dir())
        .map_err(|source| DbGraphError::io(context.dbgraph_dir(), source))?;
    fs::create_dir_all(context.snapshots_dir())
        .map_err(|source| DbGraphError::io(context.snapshots_dir(), source))?;
    fs::create_dir_all(context.instructions_dir())
        .map_err(|source| DbGraphError::io(context.instructions_dir(), source))?;

    let config_path = context.config_path();
    if config_path.exists() && !force {
        return Err(DbGraphError::invalid_config(format!(
            "{} already exists; re-run with `dbgraph init --force` to replace it",
            config_path.display()
        )));
    }

    options.config.save(&context)?;
    if options.configure_agent {
        write_instruction_fragments(&context)?;
    }

    Ok(InitSummary {
        dbgraph_dir: context.dbgraph_dir().to_path_buf(),
        instructions_dir: context.instructions_dir(),
        config_path,
    })
}

pub(crate) fn init_project_with_optional_snapshot(
    project_root: impl AsRef<Path>,
    force: bool,
    options: &InitOptions,
    snapshot_runner: impl FnOnce(&Path) -> Result<Option<SnapshotSummary>>,
) -> Result<InitRunSummary> {
    let project_root = project_root.as_ref();
    let init = init_project(project_root, force, options)?;
    let snapshot = if options.run_snapshot {
        snapshot_runner(project_root)?
    } else {
        None
    };
    Ok(InitRunSummary { init, snapshot })
}

fn write_instruction_fragments(context: &ProjectContext) -> Result<()> {
    fs::create_dir_all(context.instructions_dir())
        .map_err(|source| DbGraphError::io(context.instructions_dir(), source))?;

    for (target, content) in render_all_instruction_fragments() {
        let path = context.instructions_dir().join(target.file_name());
        fs::write(&path, content).map_err(|source| DbGraphError::io(path, source))?;
    }

    Ok(())
}

pub(crate) fn prompt_init_options() -> Result<InitOptions> {
    let mut stdout = io::stdout();
    let stdin = io::stdin();
    let mut stdin = stdin.lock().lines();

    let provider = prompt(
        &mut stdout,
        &mut stdin,
        "Database provider [postgres/mysql/sql-server/sqlite] (postgres): ",
        "postgres",
    )?;
    let provider_kind = provider
        .parse::<DatabaseProviderKind>()
        .map_err(DbGraphError::invalid_config)?;

    let source = prompt(
        &mut stdout,
        &mut stdin,
        "Connection source [DATABASE_URL/env/manual] (DATABASE_URL): ",
        "DATABASE_URL",
    )?;
    let mut database = DatabaseConfig {
        provider: provider_kind.to_string(),
        ..DatabaseConfig::default()
    };
    match source.as_str() {
        "DATABASE_URL" | "database_url" => {
            database.connection_env = Some("DATABASE_URL".to_owned());
            database.connection_string = None;
        }
        "env" | ".env" | "appsettings" => {
            let env_name = prompt(
                &mut stdout,
                &mut stdin,
                "Connection environment variable name (DATABASE_URL): ",
                "DATABASE_URL",
            )?;
            database.connection_env = Some(env_name);
            database.connection_string = None;
        }
        "manual" => {
            let connection_string = prompt(&mut stdout, &mut stdin, "Connection string: ", "")?;
            if connection_string.is_empty() {
                return Err(DbGraphError::invalid_config(
                    "manual connection source requires a connection string",
                ));
            }
            database.connection_env = None;
            database.connection_string = Some(connection_string);
        }
        _ => {
            return Err(DbGraphError::invalid_config(format!(
                "unsupported connection source `{source}`; supported values: DATABASE_URL, env, manual"
            )));
        }
    }

    let configure_agent = prompt_bool(
        &mut stdout,
        &mut stdin,
        "Generate agent instruction fragments? [Y/n]: ",
        true,
    )?;
    let run_snapshot = prompt_bool(&mut stdout, &mut stdin, "Run snapshot now? [y/N]: ", false)?;

    Ok(InitOptions {
        config: DbGraphConfig {
            database,
            ..DbGraphConfig::default()
        },
        configure_agent,
        run_snapshot,
    })
}

fn prompt(
    stdout: &mut impl Write,
    lines: &mut impl Iterator<Item = io::Result<String>>,
    message: &str,
    default: &str,
) -> Result<String> {
    stdout
        .write_all(message.as_bytes())
        .map_err(|source| DbGraphError::io("<stdout>", source))?;
    stdout
        .flush()
        .map_err(|source| DbGraphError::io("<stdout>", source))?;
    let value = lines
        .next()
        .transpose()
        .map_err(|source| DbGraphError::io("<stdin>", source))?
        .unwrap_or_default();
    let value = value.trim();
    if value.is_empty() {
        Ok(default.to_owned())
    } else {
        Ok(value.to_owned())
    }
}

fn prompt_bool(
    stdout: &mut impl Write,
    lines: &mut impl Iterator<Item = io::Result<String>>,
    message: &str,
    default: bool,
) -> Result<bool> {
    let default_text = if default { "yes" } else { "no" };
    let value = prompt(stdout, lines, message, default_text)?;
    match value.to_ascii_lowercase().as_str() {
        "y" | "yes" | "true" => Ok(true),
        "n" | "no" | "false" => Ok(false),
        _ => Err(DbGraphError::invalid_argument(format!(
            "expected yes or no, got `{value}`"
        ))),
    }
}
