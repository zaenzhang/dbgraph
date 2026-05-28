use std::env;
use std::fs;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::UNIX_EPOCH;

use dbgraph_agent_config::render_all_instruction_fragments;
use dbgraph_core::config::{DatabaseConfig, DatabaseProviderKind, DbGraphConfig};
use dbgraph_core::project::ProjectContext;
use dbgraph_core::snapshot::{now_unix_ms, SnapshotStore};
use dbgraph_core::{init_logging, version_string, DbGraphError, LogVerbosity, Result};
use dbgraph_graph::rebuild_index;
use dbgraph_provider::{ProviderConnectionConfig, ProviderRegistry};
use dbgraph_sql::{
    analyze_sql, scan_sql_files, sql_artifact_to_graph, ScanOptions, SqlDialect, SqlParser,
};
use dbgraph_storage::{GraphRepository, SqlArtifactRecord as StoredSqlArtifactRecord};
use serde::Serialize;
use tracing::debug;

fn main() -> ExitCode {
    let outcome = run(env::args().skip(1));

    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            print_error(&err);
            ExitCode::from(err.exit_code().code())
        }
    }
}

#[allow(clippy::too_many_lines)]
fn run(args: impl IntoIterator<Item = String>) -> Result<()> {
    let parsed = parse_args(args)?;
    init_logging(parsed.verbosity)?;
    debug!(verbosity = ?parsed.verbosity, "CLI logging initialized");

    match parsed.command {
        Command::Version => {
            println!("{}", version_string());
            Ok(())
        }
        Command::Help => {
            print_help();
            Ok(())
        }
        Command::Init {
            path,
            force,
            interactive,
            yes,
        } => {
            let root = match path {
                Some(path) => path,
                None => env::current_dir().map_err(|source| DbGraphError::io(".", source))?,
            };
            let options = if interactive {
                if yes {
                    InitOptions::interactive_defaults()
                } else {
                    prompt_init_options()?
                }
            } else {
                InitOptions::default()
            };
            let summary = init_project_with_optional_snapshot(&root, force, &options, |path| {
                run_snapshot(path).map(Some)
            })?;
            println!(
                "Initialized DbGraph project at {}",
                summary.init.dbgraph_dir.display()
            );
            println!("Config: {}", summary.init.config_path.display());
            if options.configure_agent {
                println!(
                    "Instruction fragments: {}",
                    summary.init.instructions_dir.display()
                );
            }
            if let Some(snapshot) = &summary.snapshot {
                print_snapshot_summary(snapshot);
            }
            Ok(())
        }
        Command::Status { path, json } => {
            let root = match path {
                Some(path) => path,
                None => env::current_dir().map_err(|source| DbGraphError::io(".", source))?,
            };
            let status = read_status(&root)?;
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&status).map_err(|source| {
                        DbGraphError::Internal {
                            message: format!("failed to serialize status: {source}"),
                        }
                    })?
                );
            } else {
                print_status(&status);
            }
            Ok(())
        }
        Command::Snapshot { path, json } => {
            let root = match path {
                Some(path) => path,
                None => env::current_dir().map_err(|source| DbGraphError::io(".", source))?,
            };
            let summary = run_snapshot(&root)?;
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&summary).map_err(|source| {
                        DbGraphError::Internal {
                            message: format!("failed to serialize snapshot summary: {source}"),
                        }
                    })?
                );
            } else {
                print_snapshot_summary(&summary);
            }
            Ok(())
        }
        Command::ValidateSql {
            path,
            sql,
            file,
            dialect,
            json,
        } => {
            let root = match path {
                Some(path) => path,
                None => env::current_dir().map_err(|source| DbGraphError::io(".", source))?,
            };
            let sql = read_sql_input(sql, file.as_deref())?;
            let report = validate_sql(&root, &sql, dialect)?;
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&report).map_err(|source| {
                        DbGraphError::Internal {
                            message: format!("failed to serialize validate-sql report: {source}"),
                        }
                    })?
                );
            } else {
                print_validate_sql_report(&report);
            }
            Ok(())
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedArgs {
    verbosity: LogVerbosity,
    command: Command,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Command {
    Version,
    Help,
    Init {
        path: Option<PathBuf>,
        force: bool,
        interactive: bool,
        yes: bool,
    },
    Status {
        path: Option<PathBuf>,
        json: bool,
    },
    Snapshot {
        path: Option<PathBuf>,
        json: bool,
    },
    ValidateSql {
        path: Option<PathBuf>,
        sql: Option<String>,
        file: Option<PathBuf>,
        dialect: SqlDialect,
        json: bool,
    },
}

#[allow(clippy::too_many_lines)]
fn parse_args(args: impl IntoIterator<Item = String>) -> Result<ParsedArgs> {
    let mut verbosity = LogVerbosity::Normal;
    let mut command = None;
    let mut pending_init = false;
    let mut pending_status = false;
    let mut pending_snapshot = false;
    let mut init_path = None;
    let mut init_force = false;
    let mut init_interactive = false;
    let mut init_yes = false;
    let mut status_path = None;
    let mut status_json = false;
    let mut snapshot_path = None;
    let mut snapshot_json = false;
    let mut pending_validate_sql = false;
    let mut validate_path = None;
    let mut validate_sql = None;
    let mut validate_file = None;
    let mut validate_dialect = SqlDialect::Postgres;
    let mut validate_json = false;
    let args = args.into_iter().collect::<Vec<_>>();
    let mut idx = 0;

    while idx < args.len() {
        let arg = args[idx].clone();
        match arg.as_str() {
            "--verbose" | "-v" => {
                if verbosity == LogVerbosity::Quiet {
                    return Err(DbGraphError::invalid_argument(
                        "`--verbose` cannot be used with `--quiet`",
                    ));
                }
                verbosity = LogVerbosity::Verbose;
            }
            "--quiet" | "-q" => {
                if verbosity == LogVerbosity::Verbose {
                    return Err(DbGraphError::invalid_argument(
                        "`--quiet` cannot be used with `--verbose`",
                    ));
                }
                verbosity = LogVerbosity::Quiet;
            }
            "--version" | "-V" | "version" => set_command(&mut command, Command::Version)?,
            "--help" | "-h" | "help" => set_command(&mut command, Command::Help)?,
            "init" => {
                set_command(
                    &mut command,
                    Command::Init {
                        path: None,
                        force: false,
                        interactive: false,
                        yes: false,
                    },
                )?;
                pending_init = true;
            }
            "status" => {
                set_command(
                    &mut command,
                    Command::Status {
                        path: None,
                        json: false,
                    },
                )?;
                pending_status = true;
            }
            "snapshot" => {
                set_command(
                    &mut command,
                    Command::Snapshot {
                        path: None,
                        json: false,
                    },
                )?;
                pending_snapshot = true;
            }
            "validate-sql" => {
                set_command(
                    &mut command,
                    Command::ValidateSql {
                        path: None,
                        sql: None,
                        file: None,
                        dialect: SqlDialect::Postgres,
                        json: false,
                    },
                )?;
                pending_validate_sql = true;
            }
            "--force" | "-f" if pending_init => {
                init_force = true;
            }
            "--interactive" | "-i" if pending_init => {
                init_interactive = true;
            }
            "--yes" | "-y" if pending_init => {
                init_yes = true;
            }
            "--json" | "-j" if pending_status => {
                status_json = true;
            }
            "--json" | "-j" if pending_snapshot => {
                snapshot_json = true;
            }
            "--json" | "-j" if pending_validate_sql => {
                validate_json = true;
            }
            "--sql" if pending_validate_sql => {
                idx += 1;
                let value = args.get(idx).ok_or_else(|| {
                    DbGraphError::invalid_argument("`--sql` requires a SQL string")
                })?;
                validate_sql = Some(value.clone());
            }
            "--file" if pending_validate_sql => {
                idx += 1;
                let value = args.get(idx).ok_or_else(|| {
                    DbGraphError::invalid_argument("`--file` requires a SQL file path")
                })?;
                validate_file = Some(PathBuf::from(value));
            }
            "--dialect" if pending_validate_sql => {
                idx += 1;
                let value = args.get(idx).ok_or_else(|| {
                    DbGraphError::invalid_argument(
                        "`--dialect` requires postgres, mysql, or generic",
                    )
                })?;
                validate_dialect = value.parse().map_err(DbGraphError::invalid_argument)?;
            }
            _ => {
                if pending_init && !arg.starts_with('-') && init_path.is_none() {
                    init_path = Some(PathBuf::from(arg));
                } else if pending_status && !arg.starts_with('-') && status_path.is_none() {
                    status_path = Some(PathBuf::from(arg));
                } else if pending_snapshot && !arg.starts_with('-') && snapshot_path.is_none() {
                    snapshot_path = Some(PathBuf::from(arg));
                } else if pending_validate_sql && !arg.starts_with('-') && validate_path.is_none() {
                    validate_path = Some(PathBuf::from(arg));
                } else {
                    return Err(DbGraphError::invalid_argument(format!(
                        "unknown command or option `{arg}`"
                    )));
                }
            }
        }
        idx += 1;
    }

    if pending_init {
        command = Some(Command::Init {
            path: init_path,
            force: init_force,
            interactive: init_interactive,
            yes: init_yes,
        });
    }
    if pending_status {
        command = Some(Command::Status {
            path: status_path,
            json: status_json,
        });
    }
    if pending_snapshot {
        command = Some(Command::Snapshot {
            path: snapshot_path,
            json: snapshot_json,
        });
    }
    if pending_validate_sql {
        if validate_sql.is_some() && validate_file.is_some() {
            return Err(DbGraphError::invalid_argument(
                "`validate-sql` accepts only one of `--sql` or `--file`",
            ));
        }
        if validate_sql.is_none() && validate_file.is_none() {
            return Err(DbGraphError::invalid_argument(
                "`validate-sql` requires `--sql <SQL>` or `--file <PATH>`",
            ));
        }
        command = Some(Command::ValidateSql {
            path: validate_path,
            sql: validate_sql,
            file: validate_file,
            dialect: validate_dialect,
            json: validate_json,
        });
    }

    Ok(ParsedArgs {
        verbosity,
        command: command.unwrap_or(Command::Help),
    })
}

fn set_command(slot: &mut Option<Command>, next: Command) -> Result<()> {
    if slot.replace(next).is_some() {
        return Err(DbGraphError::invalid_argument(
            "only one command can be supplied",
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct InitSummary {
    dbgraph_dir: PathBuf,
    config_path: PathBuf,
    instructions_dir: PathBuf,
}

#[derive(Debug, Clone)]
struct InitRunSummary {
    init: InitSummary,
    snapshot: Option<SnapshotSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct InitOptions {
    config: DbGraphConfig,
    configure_agent: bool,
    run_snapshot: bool,
}

impl InitOptions {
    fn interactive_defaults() -> Self {
        Self {
            configure_agent: true,
            run_snapshot: false,
            ..Self::default()
        }
    }
}

fn init_project(
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

fn init_project_with_optional_snapshot(
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

fn prompt_init_options() -> Result<InitOptions> {
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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct StatusReport {
    project_root: PathBuf,
    dbgraph_dir: PathBuf,
    initialized: bool,
    config_path: PathBuf,
    config_present: bool,
    provider: Option<String>,
    snapshot_count: usize,
    latest_snapshot: Option<String>,
    graph_db_path: PathBuf,
    graph_db_present: bool,
    mcp_suggestion: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct SnapshotSummary {
    project_root: PathBuf,
    snapshot_path: PathBuf,
    graph_db_path: PathBuf,
    provider: String,
    database_name: String,
    object_count: usize,
    table_count: usize,
    column_count: usize,
    edge_count: usize,
    table_profile_count: usize,
    column_profile_count: usize,
    sql_artifact_count: usize,
}

fn run_snapshot(start: impl AsRef<Path>) -> Result<SnapshotSummary> {
    let start = start.as_ref();
    let context = ProjectContext::discover_from(start)?
        .unwrap_or_else(|| ProjectContext::from_project_root(start));
    let config = DbGraphConfig::load(&context)?;
    if config.database.provider_kind()? != DatabaseProviderKind::Postgres {
        return Err(DbGraphError::invalid_config(
            "Phase 03 snapshot currently supports only provider `postgres`",
        ));
    }

    let registry = ProviderRegistry;
    let provider = registry.get(&config.database.provider).ok_or_else(|| {
        DbGraphError::invalid_config(format!(
            "provider `{}` is not registered",
            config.database.provider
        ))
    })?;
    let connection_url = resolve_connection_url(&config.database)?;
    let connection = ProviderConnectionConfig::from_url(connection_url);
    let mut snapshot = provider.snapshot(&connection)?;
    let timestamp = now_unix_ms()?;
    snapshot.created_at_unix_ms = timestamp;
    snapshot.id = format!(
        "{}:{}:{timestamp}",
        snapshot.provider, snapshot.database_name
    );

    let sql_artifacts = enrich_snapshot_with_sql(&mut snapshot, &context)?;
    let snapshot_path =
        SnapshotStore::new(&context).write_snapshot(&snapshot, config.snapshot.pretty_json)?;
    let mut repository = GraphRepository::open(context.graph_db_path())?;
    let index_summary = rebuild_index(&mut repository, &snapshot)?;
    repository.insert_sql_artifacts(&sql_artifacts)?;

    Ok(SnapshotSummary {
        project_root: context.project_root().to_path_buf(),
        snapshot_path,
        graph_db_path: context.graph_db_path(),
        provider: snapshot.provider.clone(),
        database_name: snapshot.database_name.clone(),
        object_count: index_summary.object_count,
        table_count: snapshot
            .objects
            .iter()
            .filter(|object| object.kind.as_str() == "table")
            .count(),
        column_count: snapshot
            .objects
            .iter()
            .filter(|object| object.kind.as_str() == "column")
            .count(),
        edge_count: index_summary.edge_count,
        table_profile_count: index_summary.table_profile_count,
        column_profile_count: index_summary.column_profile_count,
        sql_artifact_count: sql_artifacts.len(),
    })
}

fn enrich_snapshot_with_sql(
    snapshot: &mut dbgraph_core::model::DbSnapshot,
    context: &ProjectContext,
) -> Result<Vec<StoredSqlArtifactRecord>> {
    let sources = scan_sql_files(context.project_root(), &ScanOptions::default())?;
    let mut artifacts = Vec::new();
    for source in sources {
        let parser = SqlParser::new(SqlDialect::Postgres);
        let parsed = parser.parse(&source.raw_sql)?;
        let analysis = analyze_sql(&source.raw_sql, SqlDialect::Postgres)?;
        let source_path = source.source_path.to_string_lossy().replace('\\', "/");
        let graph = sql_artifact_to_graph(&snapshot.id, &source_path, &parsed, &analysis)?;
        snapshot.objects.push(graph.object);
        snapshot.edges.extend(graph.edges);
        artifacts.push(StoredSqlArtifactRecord {
            id: graph.artifact.id,
            snapshot_id: graph.artifact.snapshot_id,
            source_kind: graph.artifact.source_kind,
            source_path: graph.artifact.source_path,
            dialect: graph.artifact.dialect,
            fingerprint: graph.artifact.fingerprint,
            normalized_sql: graph.artifact.normalized_sql,
            ast_json: graph.artifact.ast_json,
            analysis_json: graph.artifact.analysis_json,
        });
    }
    Ok(artifacts)
}

fn resolve_connection_url(config: &DatabaseConfig) -> Result<String> {
    if let Some(env_name) = config.connection_env.as_deref() {
        if let Ok(value) = env::var(env_name) {
            if !value.trim().is_empty() {
                return Ok(value);
            }
        }
    }
    config.connection_string.clone().ok_or_else(|| {
        DbGraphError::invalid_config(
            "database connection string is missing; set DATABASE_URL or database.connectionString",
        )
    })
}

fn print_snapshot_summary(summary: &SnapshotSummary) {
    println!("DbGraph snapshot complete");
    println!("Project: {}", summary.project_root.display());
    println!("Provider: {}", summary.provider);
    println!("Database: {}", summary.database_name);
    println!("Snapshot: {}", summary.snapshot_path.display());
    println!("Graph index: {}", summary.graph_db_path.display());
    println!("Objects: {}", summary.object_count);
    println!("Tables: {}", summary.table_count);
    println!("Columns: {}", summary.column_count);
    println!("Edges: {}", summary.edge_count);
    println!("Table profiles: {}", summary.table_profile_count);
    println!("Column profiles: {}", summary.column_profile_count);
    println!("SQL artifacts: {}", summary.sql_artifact_count);
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ValidateSqlReport {
    valid: bool,
    dialect: String,
    normalized_sql: String,
    diagnostics: Vec<String>,
    unresolved: Vec<UnresolvedSqlReference>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct UnresolvedSqlReference {
    kind: String,
    name: String,
    suggestions: Vec<String>,
}

fn read_sql_input(sql: Option<String>, file: Option<&Path>) -> Result<String> {
    match (sql, file) {
        (Some(sql), None) => Ok(sql),
        (None, Some(file)) => {
            fs::read_to_string(file).map_err(|source| DbGraphError::io(file, source))
        }
        (Some(_), Some(_)) => Err(DbGraphError::invalid_argument(
            "`validate-sql` accepts only one of `--sql` or `--file`",
        )),
        (None, None) => Err(DbGraphError::invalid_argument(
            "`validate-sql` requires `--sql <SQL>` or `--file <PATH>`",
        )),
    }
}

fn validate_sql(
    start: impl AsRef<Path>,
    sql: &str,
    dialect: SqlDialect,
) -> Result<ValidateSqlReport> {
    let parsed = SqlParser::new(dialect).parse(sql)?;
    let analysis = analyze_sql(sql, dialect)?;
    let context = ProjectContext::discover_from(start.as_ref())?
        .unwrap_or_else(|| ProjectContext::from_project_root(start.as_ref()));
    let latest = SnapshotStore::new(&context).read_latest()?;
    let mut unresolved = Vec::new();
    if let Some(snapshot) = latest {
        let repository = GraphRepository::open(context.graph_db_path())?;
        for reference in analysis.references {
            if !reference_exists(&snapshot, &reference.object_name) {
                unresolved.push(UnresolvedSqlReference {
                    kind: reference.kind.as_str().to_owned(),
                    name: reference.object_name.clone(),
                    suggestions: suggest_objects(&snapshot, &repository, &reference.object_name)?,
                });
            }
        }
    }

    Ok(ValidateSqlReport {
        valid: parsed.status == dbgraph_sql::ParseStatus::Parsed,
        dialect: dialect.as_str().to_owned(),
        normalized_sql: parsed.normalized_sql,
        diagnostics: parsed
            .diagnostics
            .into_iter()
            .chain(analysis.diagnostics)
            .map(|diagnostic| diagnostic.message)
            .collect(),
        unresolved,
    })
}

fn reference_exists(snapshot: &dbgraph_core::model::DbSnapshot, name: &str) -> bool {
    let normalized = normalize_sql_name(name);
    snapshot.objects.iter().any(|object| {
        matches!(
            object.kind,
            dbgraph_core::model::DbObjectKind::Table
                | dbgraph_core::model::DbObjectKind::View
                | dbgraph_core::model::DbObjectKind::MaterializedView
                | dbgraph_core::model::DbObjectKind::Column
        ) && (normalize_sql_name(&object.full_name) == normalized
            || normalize_sql_name(&object.name) == normalized)
    })
}

fn suggest_objects(
    snapshot: &dbgraph_core::model::DbSnapshot,
    repository: &GraphRepository,
    name: &str,
) -> Result<Vec<String>> {
    let normalized = normalize_sql_name(name);
    let singular = normalized.trim_end_matches('s').to_owned();
    let plural = format!("{singular}s");
    let (table_hint, column_hint) = table_column_hint(name);
    let mut suggestions = snapshot
        .objects
        .iter()
        .filter(|object| {
            matches!(
                object.kind,
                dbgraph_core::model::DbObjectKind::Table
                    | dbgraph_core::model::DbObjectKind::View
                    | dbgraph_core::model::DbObjectKind::MaterializedView
                    | dbgraph_core::model::DbObjectKind::Column
            )
        })
        .filter(|object| {
            let object_name = normalize_sql_name(&object.name);
            if object.kind == dbgraph_core::model::DbObjectKind::Column {
                let table_matches = table_hint.as_ref().map_or(true, |hint| {
                    object
                        .table_name
                        .as_deref()
                        .is_some_and(|table| normalize_sql_name(table) == *hint)
                        || object.full_name.to_ascii_lowercase().contains(hint)
                });
                let column = column_hint.as_deref().unwrap_or(&normalized);
                table_matches
                    && (object_name == column
                        || object_name.contains(column)
                        || edit_distance(&object_name, column) <= 2)
            } else {
                object_name == singular || object_name == plural || object_name.contains(&singular)
            }
        })
        .map(|object| object.full_name.clone())
        .collect::<Vec<_>>();
    for object in repository.search_objects(&normalized)? {
        if !suggestions.contains(&object.full_name) {
            suggestions.push(object.full_name);
        }
    }
    suggestions.sort();
    suggestions.dedup();
    suggestions.truncate(5);
    Ok(suggestions)
}

fn table_column_hint(name: &str) -> (Option<String>, Option<String>) {
    let parts = name
        .split('.')
        .map(normalize_sql_name)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    match parts.as_slice() {
        [table, column] | [_, table, column] => (Some(table.clone()), Some(column.clone())),
        [column] => (None, Some(column.clone())),
        _ => (None, None),
    }
}

fn edit_distance(left: &str, right: &str) -> usize {
    let mut costs = (0..=right.len()).collect::<Vec<_>>();
    for (left_idx, left_char) in left.chars().enumerate() {
        let mut previous = costs[0];
        costs[0] = left_idx + 1;
        for (right_idx, right_char) in right.chars().enumerate() {
            let insert = costs[right_idx + 1] + 1;
            let delete = costs[right_idx] + 1;
            let replace = previous + usize::from(left_char != right_char);
            previous = costs[right_idx + 1];
            costs[right_idx + 1] = insert.min(delete).min(replace);
        }
    }
    *costs.last().unwrap_or(&0)
}

fn normalize_sql_name(value: &str) -> String {
    value
        .rsplit('.')
        .next()
        .unwrap_or(value)
        .trim_matches('"')
        .trim_matches('`')
        .to_ascii_lowercase()
}

fn print_validate_sql_report(report: &ValidateSqlReport) {
    println!("SQL validation");
    println!("Dialect: {}", report.dialect);
    println!("Parse: {}", if report.valid { "valid" } else { "invalid" });
    if !report.diagnostics.is_empty() {
        println!("Diagnostics:");
        for diagnostic in &report.diagnostics {
            println!("  - {diagnostic}");
        }
    }
    if !report.unresolved.is_empty() {
        println!("Unresolved references:");
        for item in &report.unresolved {
            if item.suggestions.is_empty() {
                println!("  - {} {}", item.kind, item.name);
            } else {
                println!(
                    "  - {} {} (suggestions: {})",
                    item.kind,
                    item.name,
                    item.suggestions.join(", ")
                );
            }
        }
    }
}

fn read_status(start: impl AsRef<Path>) -> Result<StatusReport> {
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
        mcp_suggestion: "Run `dbgraph serve --mcp` after MCP support is installed.".to_owned(),
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

fn print_status(status: &StatusReport) {
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

fn print_error(err: &DbGraphError) {
    eprintln!("error: {err}");
    eprintln!("Run `dbgraph --help` for usage.");
    debug!(error = ?err, "command failed");
}

fn print_help() {
    println!(
        "\
DbGraph

Usage:
  dbgraph [OPTIONS] init [PATH] [--force] [-i|--interactive] [--yes]
  dbgraph [OPTIONS] status [PATH] [--json]
  dbgraph [OPTIONS] snapshot [PATH] [--json]
  dbgraph [OPTIONS] validate-sql [PATH] (--sql SQL | --file FILE) [--dialect postgres|mysql|generic] [--json]
  dbgraph [OPTIONS] --version
  dbgraph [OPTIONS] --help

Options:
  -v, --verbose      Show debug diagnostics
  -q, --quiet        Show errors only
  -V, --version      Print version
  -h, --help         Print help
  -f, --force        Replace existing init config when used with init
  -i, --interactive  Prompt for init options
  -y, --yes          Use interactive init defaults without prompts
  -j, --json         Print status as JSON

Commands:
  init       Initialize .dbgraph project state
  status     Show local project status
  snapshot   Capture PostgreSQL schema into JSON and local SQLite index
  validate-sql Parse SQL and validate references against the local graph index"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(items: &[&str]) -> Result<ParsedArgs> {
        parse_args(items.iter().map(ToString::to_string))
    }

    #[test]
    fn parses_verbose_version() {
        let parsed = parse(&["--verbose", "--version"]).expect("args should parse");

        assert_eq!(parsed.verbosity, LogVerbosity::Verbose);
        assert_eq!(parsed.command, Command::Version);
    }

    #[test]
    fn parses_quiet_help() {
        let parsed = parse(&["--quiet", "help"]).expect("args should parse");

        assert_eq!(parsed.verbosity, LogVerbosity::Quiet);
        assert_eq!(parsed.command, Command::Help);
    }

    #[test]
    fn parses_init_defaults_to_current_directory() {
        let parsed = parse(&["init"]).expect("args should parse");

        assert_eq!(
            parsed.command,
            Command::Init {
                path: None,
                force: false,
                interactive: false,
                yes: false
            }
        );
    }

    #[test]
    fn parses_init_path_and_force() {
        let parsed = parse(&["init", "sample-project", "--force"]).expect("args should parse");

        assert_eq!(
            parsed.command,
            Command::Init {
                path: Some(PathBuf::from("sample-project")),
                force: true,
                interactive: false,
                yes: false
            }
        );
    }

    #[test]
    fn parses_interactive_yes_init() {
        let parsed = parse(&["init", "-i", "--yes"]).expect("args should parse");

        assert_eq!(
            parsed.command,
            Command::Init {
                path: None,
                force: false,
                interactive: true,
                yes: true
            }
        );
    }

    #[test]
    fn parses_status_json() {
        let parsed = parse(&["status", "--json", "sample-project"]).expect("args should parse");

        assert_eq!(
            parsed.command,
            Command::Status {
                path: Some(PathBuf::from("sample-project")),
                json: true
            }
        );
    }

    #[test]
    fn parses_snapshot_command() {
        let parsed = parse(&["snapshot", "--json"]).expect("args should parse");

        assert_eq!(
            parsed.command,
            Command::Snapshot {
                path: None,
                json: true
            }
        );
    }

    #[test]
    fn parses_validate_sql_from_string() {
        let parsed = parse(&[
            "validate-sql",
            "--sql",
            "select * from users",
            "--dialect",
            "postgres",
            "--json",
        ])
        .expect("args should parse");

        assert_eq!(
            parsed.command,
            Command::ValidateSql {
                path: None,
                sql: Some("select * from users".to_owned()),
                file: None,
                dialect: dbgraph_sql::SqlDialect::Postgres,
                json: true
            }
        );
    }

    #[test]
    fn validate_sql_suggests_known_table_without_database_connection() {
        let temp = TempProject::new();
        init_project(&temp.root, false, &InitOptions::default()).expect("init should succeed");
        let context = ProjectContext::from_project_root(&temp.root);
        let mut snapshot = dbgraph_core::model::DbSnapshot::new("s1", "postgres", "app", 1);
        snapshot.objects.push(dbgraph_core::model::DbObject::new(
            "table:public.users",
            dbgraph_core::model::DbObjectKind::Table,
            "public.users",
        ));
        SnapshotStore::new(&context)
            .write_snapshot(&snapshot, true)
            .expect("snapshot should write");
        let mut repo = GraphRepository::open(context.graph_db_path()).expect("repo should open");
        repo.rebuild_snapshot(&snapshot)
            .expect("graph index should write");

        let report = validate_sql(
            &temp.root,
            "select * from user",
            dbgraph_sql::SqlDialect::Postgres,
        )
        .expect("validate should not require DB connection");

        assert!(report.valid);
        assert!(report.unresolved.iter().any(
            |item| item.name == "user" && item.suggestions.contains(&"public.users".to_owned())
        ));
    }

    #[test]
    fn validate_sql_reports_unresolved_columns_with_suggestions() {
        let temp = TempProject::new();
        init_project(&temp.root, false, &InitOptions::default()).expect("init should succeed");
        let context = ProjectContext::from_project_root(&temp.root);
        let mut snapshot = dbgraph_core::model::DbSnapshot::new("s1", "postgres", "app", 1);
        snapshot.objects.push(dbgraph_core::model::DbObject::new(
            "table:public.users",
            dbgraph_core::model::DbObjectKind::Table,
            "public.users",
        ));
        let mut email = dbgraph_core::model::DbObject::new(
            "column:public.users.email",
            dbgraph_core::model::DbObjectKind::Column,
            "public.users.email",
        );
        email.table_name = Some("users".to_owned());
        email.column_name = Some("email".to_owned());
        snapshot.objects.push(email);
        SnapshotStore::new(&context)
            .write_snapshot(&snapshot, true)
            .expect("snapshot should write");
        let mut repo = GraphRepository::open(context.graph_db_path()).expect("repo should open");
        repo.rebuild_snapshot(&snapshot)
            .expect("graph index should write");

        let report = validate_sql(
            &temp.root,
            "select * from public.users u where u.emali = 'x'",
            dbgraph_sql::SqlDialect::Postgres,
        )
        .expect("validate should inspect local graph index");

        assert!(report
            .unresolved
            .iter()
            .any(|item| item.name == "public.users.emali"
                && item.suggestions.contains(&"public.users.email".to_owned())));
    }

    #[test]
    fn snapshot_requires_initialized_project() {
        let temp = TempProject::new();
        fs::create_dir_all(&temp.root).expect("temp root should exist");

        let err = run(["snapshot".to_owned(), temp.root.display().to_string()])
            .expect_err("snapshot should require config");

        assert!(err.to_string().contains("Run `dbgraph init` first"));
    }

    #[test]
    fn init_creates_expected_layout() {
        let temp = TempProject::new();

        let summary =
            init_project(&temp.root, false, &InitOptions::default()).expect("init should succeed");
        let context = ProjectContext::from_project_root(&temp.root);

        assert_eq!(summary.dbgraph_dir, context.dbgraph_dir());
        assert_eq!(summary.instructions_dir, context.instructions_dir());
        assert!(context.dbgraph_dir().is_dir());
        assert!(context.snapshots_dir().is_dir());
        assert!(context.instructions_dir().is_dir());
        assert!(context.config_path().is_file());
    }

    #[test]
    fn init_does_not_overwrite_existing_config_without_force() {
        let temp = TempProject::new();
        init_project(&temp.root, false, &InitOptions::default())
            .expect("first init should succeed");
        let context = ProjectContext::from_project_root(&temp.root);
        let custom = "{ \"custom\": true }\n";
        fs::write(context.config_path(), custom).expect("custom config should be written");

        let err = init_project(&temp.root, false, &InitOptions::default())
            .expect_err("second init should fail");
        let stored = fs::read_to_string(context.config_path()).expect("config should be readable");

        assert!(err.to_string().contains("--force"));
        assert_eq!(stored, custom);
    }

    #[test]
    fn init_force_replaces_existing_config() {
        let temp = TempProject::new();
        init_project(&temp.root, false, &InitOptions::default())
            .expect("first init should succeed");
        let context = ProjectContext::from_project_root(&temp.root);
        fs::write(context.config_path(), "{ \"custom\": true }\n")
            .expect("custom config should be written");

        init_project(&temp.root, true, &InitOptions::default()).expect("force init should succeed");
        let config = DbGraphConfig::load(&context).expect("default config should load");

        assert!(!config.snapshot.sample_rows);
        assert!(!config.security.store_raw_data);
    }

    #[test]
    fn interactive_yes_writes_instruction_fragments() {
        let temp = TempProject::new();
        init_project(&temp.root, false, &InitOptions::interactive_defaults())
            .expect("interactive init should succeed");
        let context = ProjectContext::from_project_root(&temp.root);

        assert!(context
            .instructions_dir()
            .join("AGENTS.md.fragment")
            .is_file());
        assert!(context
            .instructions_dir()
            .join("CLAUDE.md.fragment")
            .is_file());
        assert!(context.instructions_dir().join("dbgraph.mdc").is_file());
        let config = DbGraphConfig::load(&context).expect("config should load");
        assert!(config.database.connection_string.is_none());
        assert_eq!(
            config.database.connection_env.as_deref(),
            Some("DATABASE_URL")
        );
    }

    #[test]
    fn init_with_run_snapshot_invokes_snapshot_after_project_files_exist() {
        let temp = TempProject::new();
        let options = InitOptions {
            run_snapshot: true,
            ..InitOptions::default()
        };

        let summary = init_project_with_optional_snapshot(&temp.root, false, &options, |root| {
            let context = ProjectContext::from_project_root(root);
            assert!(context.config_path().is_file());
            Ok(None)
        })
        .expect("init should succeed");

        assert!(summary.snapshot.is_none());
    }

    #[test]
    fn status_reports_uninitialized_project() {
        let temp = TempProject::new();
        fs::create_dir_all(&temp.root).expect("temp root should exist");

        let status = read_status(&temp.root).expect("status should load");

        assert!(!status.initialized);
        assert!(!status.config_present);
        assert_eq!(status.snapshot_count, 0);
    }

    #[test]
    fn status_reports_initialized_project() {
        let temp = TempProject::new();
        init_project(&temp.root, false, &InitOptions::default()).expect("init should succeed");
        let context = ProjectContext::from_project_root(&temp.root);
        fs::write(context.snapshots_dir().join("2026-01-01.json"), "{}")
            .expect("snapshot should be written");

        let status = read_status(&temp.root).expect("status should load");

        assert!(status.initialized);
        assert_eq!(status.provider.as_deref(), Some("postgres"));
        assert_eq!(status.snapshot_count, 1);
        assert_eq!(status.latest_snapshot.as_deref(), Some("2026-01-01.json"));
        assert!(!status.graph_db_present);
    }

    #[test]
    fn rejects_conflicting_verbosity() {
        let err = parse(&["--quiet", "--verbose"]).expect_err("conflict should fail");

        assert_eq!(err.exit_code().code(), 2);
        assert!(err.to_string().contains("cannot be used"));
    }

    #[test]
    fn rejects_unknown_argument() {
        let err = parse(&["--bad"]).expect_err("unknown arg should fail");

        assert_eq!(err.exit_code().code(), 2);
        assert!(err.to_string().contains("--bad"));
    }

    struct TempProject {
        root: PathBuf,
    }

    impl TempProject {
        fn new() -> Self {
            let unique = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock should be valid")
                .as_nanos();
            let root = env::temp_dir().join(format!(
                "dbgraph-cli-init-test-{}-{unique}",
                std::process::id()
            ));
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
