//! CLI argument parsing.

use std::path::{Path, PathBuf};

use dbgraph_agent_config::{parse_agent_kinds, AgentKind};
use dbgraph_core::profiling::ProfilingMode;
use dbgraph_core::{DbGraphError, LogVerbosity, Result};
use dbgraph_graph::analysis::{AnalysisScope, FindingSeverity};
use dbgraph_graph::relations::Direction;
use dbgraph_sql::SqlDialect;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ParsedArgs {
    pub(crate) verbosity: LogVerbosity,
    pub(crate) command: Command,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Command {
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
    Doctor {
        path: Option<PathBuf>,
        json: bool,
        check_db: bool,
    },
    Snapshot {
        path: Option<PathBuf>,
        json: bool,
        profile: Option<ProfilingMode>,
        max_rows_per_table: Option<u32>,
        store_raw_samples: bool,
    },
    Sync {
        path: Option<PathBuf>,
        json: bool,
    },
    Benchmark {
        tables: usize,
        columns_per_table: usize,
        json: bool,
    },
    BenchmarkAgent {
        path: Option<PathBuf>,
        scenario: String,
        format: AgentBenchmarkFormat,
        output: Option<PathBuf>,
    },
    ValidateSql {
        path: Option<PathBuf>,
        sql: Option<String>,
        file: Option<PathBuf>,
        dialect: SqlDialect,
        json: bool,
    },
    Search {
        path: Option<PathBuf>,
        query: String,
        kind: Option<String>,
        json: bool,
    },
    Table {
        path: Option<PathBuf>,
        table: String,
        json: bool,
    },
    Relations {
        path: Option<PathBuf>,
        object: String,
        depth: usize,
        direction: Direction,
        json: bool,
    },
    Context {
        path: Option<PathBuf>,
        query: String,
        token_budget: usize,
        json: bool,
    },
    Diff {
        path: Option<PathBuf>,
        json: bool,
    },
    Impact {
        path: Option<PathBuf>,
        object: String,
        depth: usize,
        json: bool,
    },
    Analyze {
        path: Option<PathBuf>,
        scope: AnalysisScope,
        json: bool,
        format: AnalysisOutputFormat,
        output: Option<PathBuf>,
        include_suppressed: bool,
        suppressions: Option<PathBuf>,
        fail_on: Option<FindingSeverity>,
        fail_on_new: Option<FindingSeverity>,
        baseline: Option<PathBuf>,
    },
    Install {
        targets: Vec<AgentKind>,
        location: Option<PathBuf>,
        yes: bool,
        dry_run: bool,
        print_config: bool,
    },
    Uninstall {
        targets: Vec<AgentKind>,
        location: Option<PathBuf>,
        dry_run: bool,
    },
    ServeMcp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AnalysisOutputFormat {
    Text,
    Json,
    Markdown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AgentBenchmarkFormat {
    Text,
    Json,
    Markdown,
}

impl std::str::FromStr for AgentBenchmarkFormat {
    type Err = String;

    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "text" => Ok(Self::Text),
            "json" => Ok(Self::Json),
            "markdown" | "md" => Ok(Self::Markdown),
            _ => Err("agent benchmark format must be text, json, or markdown".to_owned()),
        }
    }
}

impl std::str::FromStr for AnalysisOutputFormat {
    type Err = String;

    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "text" => Ok(Self::Text),
            "json" => Ok(Self::Json),
            "markdown" | "md" => Ok(Self::Markdown),
            _ => Err("analysis format must be text, json, or markdown".to_owned()),
        }
    }
}

#[allow(clippy::too_many_lines)]
pub(crate) fn parse_args(args: impl IntoIterator<Item = String>) -> Result<ParsedArgs> {
    let mut verbosity = LogVerbosity::Normal;
    let mut command = None;
    let mut pending_init = false;
    let mut pending_status = false;
    let mut pending_doctor = false;
    let mut pending_snapshot = false;
    let mut pending_sync = false;
    let mut pending_benchmark = false;
    let mut pending_benchmark_agent = false;
    let mut init_path = None;
    let mut init_force = false;
    let mut init_interactive = false;
    let mut init_yes = false;
    let mut status_path = None;
    let mut status_json = false;
    let mut doctor_path = None;
    let mut doctor_json = false;
    let mut doctor_check_db = false;
    let mut snapshot_path = None;
    let mut snapshot_json = false;
    let mut snapshot_profile = None;
    let mut snapshot_max_rows_per_table = None;
    let mut snapshot_store_raw_samples = false;
    let mut sync_path = None;
    let mut sync_json = false;
    let mut benchmark_tables = 1_000_usize;
    let mut benchmark_columns_per_table = 4_usize;
    let mut benchmark_json = false;
    let mut benchmark_agent_path = None;
    let mut benchmark_agent_scenario = "teashop".to_owned();
    let mut benchmark_agent_format = AgentBenchmarkFormat::Text;
    let mut benchmark_agent_output = None;
    let mut pending_validate_sql = false;
    let mut validate_path = None;
    let mut validate_sql = None;
    let mut validate_file = None;
    let mut validate_dialect = SqlDialect::Postgres;
    let mut validate_json = false;
    let mut pending_search = false;
    let mut search_positionals = Vec::new();
    let mut search_kind = None;
    let mut search_json = false;
    let mut pending_table = false;
    let mut table_positionals = Vec::new();
    let mut table_json = false;
    let mut pending_relations = false;
    let mut relations_positionals = Vec::new();
    let mut relations_depth = 1_usize;
    let mut relations_direction = Direction::Both;
    let mut relations_json = false;
    let mut pending_context = false;
    let mut context_positionals = Vec::new();
    let mut context_budget = 800_usize;
    let mut context_json = false;
    let mut pending_diff = false;
    let mut diff_path = None;
    let mut diff_json = false;
    let mut pending_impact = false;
    let mut pending_analyze = false;
    let mut impact_positionals = Vec::new();
    let mut impact_depth = 2_usize;
    let mut impact_json = false;
    let mut analyze_path = None;
    let mut analyze_scope = AnalysisScope::All;
    let mut analyze_json = false;
    let mut analyze_format = AnalysisOutputFormat::Text;
    let mut analyze_output = None;
    let mut analyze_include_suppressed = false;
    let mut analyze_suppressions = None;
    let mut analyze_fail_on = None;
    let mut analyze_fail_on_new = None;
    let mut analyze_baseline = None;
    let mut pending_install = false;
    let mut install_target_raw: Option<String> = None;
    let mut install_location = None;
    let mut install_yes = false;
    let mut install_dry_run = false;
    let mut install_print_config = false;
    let mut pending_uninstall = false;
    let mut uninstall_target_raw: Option<String> = None;
    let mut uninstall_location = None;
    let mut uninstall_dry_run = false;
    let mut pending_serve = false;
    let mut serve_mcp = false;
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
            "doctor" => {
                set_command(
                    &mut command,
                    Command::Doctor {
                        path: None,
                        json: false,
                        check_db: false,
                    },
                )?;
                pending_doctor = true;
            }
            "snapshot" => {
                set_command(
                    &mut command,
                    Command::Snapshot {
                        path: None,
                        json: false,
                        profile: None,
                        max_rows_per_table: None,
                        store_raw_samples: false,
                    },
                )?;
                pending_snapshot = true;
            }
            "sync" => {
                set_command(
                    &mut command,
                    Command::Sync {
                        path: None,
                        json: false,
                    },
                )?;
                pending_sync = true;
            }
            "benchmark" => {
                set_command(
                    &mut command,
                    Command::Benchmark {
                        tables: 1_000,
                        columns_per_table: 4,
                        json: false,
                    },
                )?;
                pending_benchmark = true;
            }
            "benchmark-agent" => {
                set_command(
                    &mut command,
                    Command::BenchmarkAgent {
                        path: None,
                        scenario: "teashop".to_owned(),
                        format: AgentBenchmarkFormat::Text,
                        output: None,
                    },
                )?;
                pending_benchmark_agent = true;
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
            "search" => {
                set_command(
                    &mut command,
                    Command::Search {
                        path: None,
                        query: String::new(),
                        kind: None,
                        json: false,
                    },
                )?;
                pending_search = true;
            }
            "table" => {
                set_command(
                    &mut command,
                    Command::Table {
                        path: None,
                        table: String::new(),
                        json: false,
                    },
                )?;
                pending_table = true;
            }
            "relations" => {
                set_command(
                    &mut command,
                    Command::Relations {
                        path: None,
                        object: String::new(),
                        depth: 1,
                        direction: Direction::Both,
                        json: false,
                    },
                )?;
                pending_relations = true;
            }
            "context" => {
                set_command(
                    &mut command,
                    Command::Context {
                        path: None,
                        query: String::new(),
                        token_budget: 800,
                        json: false,
                    },
                )?;
                pending_context = true;
            }
            "diff" => {
                set_command(
                    &mut command,
                    Command::Diff {
                        path: None,
                        json: false,
                    },
                )?;
                pending_diff = true;
            }
            "impact" => {
                set_command(
                    &mut command,
                    Command::Impact {
                        path: None,
                        object: String::new(),
                        depth: 2,
                        json: false,
                    },
                )?;
                pending_impact = true;
            }
            "analyze" | "analyse" => {
                set_command(
                    &mut command,
                    Command::Analyze {
                        path: None,
                        scope: AnalysisScope::All,
                        json: false,
                        format: AnalysisOutputFormat::Text,
                        output: None,
                        include_suppressed: false,
                        suppressions: None,
                        fail_on: None,
                        fail_on_new: None,
                        baseline: None,
                    },
                )?;
                pending_analyze = true;
            }
            "install" => {
                set_command(
                    &mut command,
                    Command::Install {
                        targets: AgentKind::all().to_vec(),
                        location: None,
                        yes: false,
                        dry_run: false,
                        print_config: false,
                    },
                )?;
                pending_install = true;
            }
            "uninstall" => {
                set_command(
                    &mut command,
                    Command::Uninstall {
                        targets: AgentKind::all().to_vec(),
                        location: None,
                        dry_run: false,
                    },
                )?;
                pending_uninstall = true;
            }
            "serve" => {
                set_command(&mut command, Command::ServeMcp)?;
                pending_serve = true;
            }
            "--mcp" if pending_serve => {
                serve_mcp = true;
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
            "--json" | "-j" if pending_doctor => {
                doctor_json = true;
            }
            "--check-db" if pending_doctor => {
                doctor_check_db = true;
            }
            "--json" | "-j" if pending_snapshot => {
                snapshot_json = true;
            }
            "--profile" if pending_snapshot => {
                idx += 1;
                let value = args.get(idx).ok_or_else(|| {
                    DbGraphError::invalid_argument("`--profile` requires a value")
                })?;
                snapshot_profile = Some(
                    value
                        .parse::<ProfilingMode>()
                        .map_err(DbGraphError::invalid_argument)?,
                );
            }
            "--max-rows-per-table" if pending_snapshot => {
                idx += 1;
                let value = args.get(idx).ok_or_else(|| {
                    DbGraphError::invalid_argument("`--max-rows-per-table` requires a value")
                })?;
                snapshot_max_rows_per_table = Some(parse_positive_u32(
                    value,
                    "`--max-rows-per-table` requires a positive integer",
                )?);
            }
            "--store-raw-samples" if pending_snapshot => {
                snapshot_store_raw_samples = true;
            }
            "--json" | "-j" if pending_sync => {
                sync_json = true;
            }
            "--json" | "-j" if pending_benchmark => {
                benchmark_json = true;
            }
            "--scenario" if pending_benchmark_agent => {
                idx += 1;
                benchmark_agent_scenario.clone_from(args.get(idx).ok_or_else(|| {
                    DbGraphError::invalid_argument("`--scenario` requires a value")
                })?);
            }
            "--format" if pending_benchmark_agent => {
                idx += 1;
                let value = args.get(idx).ok_or_else(|| {
                    DbGraphError::invalid_argument("`--format` requires text, json, or markdown")
                })?;
                benchmark_agent_format = value.parse().map_err(DbGraphError::invalid_argument)?;
            }
            "--output" if pending_benchmark_agent => {
                idx += 1;
                let value = args.get(idx).ok_or_else(|| {
                    DbGraphError::invalid_argument("`--output` requires a file path")
                })?;
                benchmark_agent_output = Some(PathBuf::from(value));
            }
            "--tables" if pending_benchmark => {
                idx += 1;
                let value = args
                    .get(idx)
                    .ok_or_else(|| DbGraphError::invalid_argument("`--tables` requires a value"))?;
                benchmark_tables =
                    parse_positive_usize(value, "`--tables` requires a positive integer")?;
            }
            "--columns-per-table" if pending_benchmark => {
                idx += 1;
                let value = args.get(idx).ok_or_else(|| {
                    DbGraphError::invalid_argument("`--columns-per-table` requires a value")
                })?;
                benchmark_columns_per_table = parse_positive_usize(
                    value,
                    "`--columns-per-table` requires a positive integer",
                )?;
            }
            "--json" | "-j" if pending_validate_sql => {
                validate_json = true;
            }
            "--json" | "-j" if pending_search => search_json = true,
            "--json" | "-j" if pending_table => table_json = true,
            "--json" | "-j" if pending_relations => relations_json = true,
            "--json" | "-j" if pending_context => context_json = true,
            "--json" | "-j" if pending_diff => diff_json = true,
            "--json" | "-j" if pending_impact => impact_json = true,
            "--json" | "-j" if pending_analyze => {
                analyze_json = true;
                analyze_format = AnalysisOutputFormat::Json;
            }
            "--yes" | "-y" if pending_install => install_yes = true,
            "--dry-run" if pending_install => install_dry_run = true,
            "--dry-run" if pending_uninstall => uninstall_dry_run = true,
            "--print-config" if pending_install => install_print_config = true,
            "--target" | "--targets" if pending_install => {
                idx += 1;
                let value = args
                    .get(idx)
                    .ok_or_else(|| DbGraphError::invalid_argument("`--target` requires a value"))?;
                install_target_raw = Some(value.clone());
            }
            "--target" | "--targets" if pending_uninstall => {
                idx += 1;
                let value = args
                    .get(idx)
                    .ok_or_else(|| DbGraphError::invalid_argument("`--target` requires a value"))?;
                uninstall_target_raw = Some(value.clone());
            }
            "--location" if pending_install => {
                idx += 1;
                let value = args.get(idx).ok_or_else(|| {
                    DbGraphError::invalid_argument("`--location` requires a directory")
                })?;
                install_location = Some(PathBuf::from(value));
            }
            "--location" if pending_uninstall => {
                idx += 1;
                let value = args.get(idx).ok_or_else(|| {
                    DbGraphError::invalid_argument("`--location` requires a directory")
                })?;
                uninstall_location = Some(PathBuf::from(value));
            }
            value if pending_install && value.starts_with("--target=") => {
                install_target_raw = Some(value["--target=".len()..].to_owned());
            }
            value if pending_install && value.starts_with("--targets=") => {
                install_target_raw = Some(value["--targets=".len()..].to_owned());
            }
            value if pending_uninstall && value.starts_with("--target=") => {
                uninstall_target_raw = Some(value["--target=".len()..].to_owned());
            }
            value if pending_uninstall && value.starts_with("--targets=") => {
                uninstall_target_raw = Some(value["--targets=".len()..].to_owned());
            }
            value if pending_install && value.starts_with("--location=") => {
                install_location = Some(PathBuf::from(&value["--location=".len()..]));
            }
            value if pending_uninstall && value.starts_with("--location=") => {
                uninstall_location = Some(PathBuf::from(&value["--location=".len()..]));
            }
            "--kind" if pending_search => {
                idx += 1;
                let value = args
                    .get(idx)
                    .ok_or_else(|| DbGraphError::invalid_argument("`--kind` requires a value"))?;
                search_kind = Some(value.clone());
            }
            "--depth" if pending_relations => {
                idx += 1;
                let value = args
                    .get(idx)
                    .ok_or_else(|| DbGraphError::invalid_argument("`--depth` requires 1 or 2"))?;
                relations_depth = parse_depth(value)?;
            }
            "--direction" if pending_relations => {
                idx += 1;
                let value = args.get(idx).ok_or_else(|| {
                    DbGraphError::invalid_argument(
                        "`--direction` requires incoming, outgoing, or both",
                    )
                })?;
                relations_direction = parse_direction(value)?;
            }
            "--tokens" | "--token-budget" if pending_context => {
                idx += 1;
                let value = args.get(idx).ok_or_else(|| {
                    DbGraphError::invalid_argument("`--tokens` requires a positive integer")
                })?;
                context_budget = value.parse::<usize>().map_err(|_| {
                    DbGraphError::invalid_argument("`--tokens` requires a positive integer")
                })?;
            }
            "--depth" if pending_impact => {
                idx += 1;
                let value = args
                    .get(idx)
                    .ok_or_else(|| DbGraphError::invalid_argument("`--depth` requires 1 or 2"))?;
                impact_depth = parse_depth(value)?;
            }
            "--scope" if pending_analyze => {
                idx += 1;
                let value = args.get(idx).ok_or_else(|| {
                    DbGraphError::invalid_argument(
                        "`--scope` requires all, risk, quality, or performance",
                    )
                })?;
                analyze_scope = value.parse().map_err(DbGraphError::invalid_argument)?;
            }
            "--format" if pending_analyze => {
                idx += 1;
                let value = args.get(idx).ok_or_else(|| {
                    DbGraphError::invalid_argument("`--format` requires text, json, or markdown")
                })?;
                analyze_format = value.parse().map_err(DbGraphError::invalid_argument)?;
            }
            "--output" if pending_analyze => {
                idx += 1;
                let value = args.get(idx).ok_or_else(|| {
                    DbGraphError::invalid_argument("`--output` requires a file path")
                })?;
                analyze_output = Some(PathBuf::from(value));
            }
            "--include-suppressed" if pending_analyze => {
                analyze_include_suppressed = true;
            }
            "--suppressions" if pending_analyze => {
                idx += 1;
                let value = args.get(idx).ok_or_else(|| {
                    DbGraphError::invalid_argument("`--suppressions` requires a file path")
                })?;
                analyze_suppressions = Some(PathBuf::from(value));
            }
            "--fail-on" if pending_analyze => {
                idx += 1;
                let value = args.get(idx).ok_or_else(|| {
                    DbGraphError::invalid_argument("`--fail-on` requires a severity")
                })?;
                analyze_fail_on = Some(value.parse().map_err(DbGraphError::invalid_argument)?);
            }
            "--fail-on-new" if pending_analyze => {
                idx += 1;
                let value = args.get(idx).ok_or_else(|| {
                    DbGraphError::invalid_argument("`--fail-on-new` requires a severity")
                })?;
                analyze_fail_on_new = Some(value.parse().map_err(DbGraphError::invalid_argument)?);
            }
            "--baseline" if pending_analyze => {
                idx += 1;
                let value = args.get(idx).ok_or_else(|| {
                    DbGraphError::invalid_argument("`--baseline` requires a file path")
                })?;
                analyze_baseline = Some(PathBuf::from(value));
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
                } else if pending_doctor && !arg.starts_with('-') && doctor_path.is_none() {
                    doctor_path = Some(PathBuf::from(arg));
                } else if pending_snapshot && !arg.starts_with('-') && snapshot_path.is_none() {
                    snapshot_path = Some(PathBuf::from(arg));
                } else if pending_sync && !arg.starts_with('-') && sync_path.is_none() {
                    sync_path = Some(PathBuf::from(arg));
                } else if pending_benchmark_agent
                    && !arg.starts_with('-')
                    && benchmark_agent_path.is_none()
                {
                    benchmark_agent_path = Some(PathBuf::from(arg));
                } else if pending_validate_sql && !arg.starts_with('-') && validate_path.is_none() {
                    validate_path = Some(PathBuf::from(arg));
                } else if pending_search && !arg.starts_with('-') {
                    search_positionals.push(arg);
                } else if pending_table && !arg.starts_with('-') {
                    table_positionals.push(arg);
                } else if pending_relations && !arg.starts_with('-') {
                    relations_positionals.push(arg);
                } else if pending_context && !arg.starts_with('-') {
                    context_positionals.push(arg);
                } else if pending_diff && !arg.starts_with('-') && diff_path.is_none() {
                    if arg != "latest" && arg != "previous" {
                        diff_path = Some(PathBuf::from(arg));
                    }
                } else if pending_impact && !arg.starts_with('-') {
                    impact_positionals.push(arg);
                } else if pending_analyze && !arg.starts_with('-') && analyze_path.is_none() {
                    analyze_path = Some(PathBuf::from(arg));
                } else if pending_install || pending_uninstall {
                    return Err(DbGraphError::invalid_argument(format!(
                        "unknown install option `{arg}`"
                    )));
                } else if pending_serve {
                    return Err(DbGraphError::invalid_argument(
                        "`serve` currently supports only `--mcp`",
                    ));
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
    if pending_doctor {
        command = Some(Command::Doctor {
            path: doctor_path,
            json: doctor_json,
            check_db: doctor_check_db,
        });
    }
    if pending_snapshot {
        command = Some(Command::Snapshot {
            path: snapshot_path,
            json: snapshot_json,
            profile: snapshot_profile,
            max_rows_per_table: snapshot_max_rows_per_table,
            store_raw_samples: snapshot_store_raw_samples,
        });
    }
    if pending_sync {
        command = Some(Command::Sync {
            path: sync_path,
            json: sync_json,
        });
    }
    if pending_benchmark {
        command = Some(Command::Benchmark {
            tables: benchmark_tables,
            columns_per_table: benchmark_columns_per_table,
            json: benchmark_json,
        });
    }
    if pending_benchmark_agent {
        if benchmark_agent_scenario != "teashop" {
            return Err(DbGraphError::invalid_argument(
                "`benchmark-agent` currently supports only `--scenario teashop`",
            ));
        }
        command = Some(Command::BenchmarkAgent {
            path: benchmark_agent_path,
            scenario: benchmark_agent_scenario,
            format: benchmark_agent_format,
            output: benchmark_agent_output,
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
    if pending_search {
        let (path, query) = split_optional_path_and_query(&search_positionals, "search")?;
        command = Some(Command::Search {
            path,
            query,
            kind: search_kind,
            json: search_json,
        });
    }
    if pending_table {
        let (path, table) = split_optional_path_and_query(&table_positionals, "table")?;
        command = Some(Command::Table {
            path,
            table,
            json: table_json,
        });
    }
    if pending_relations {
        let (path, object) = split_optional_path_and_query(&relations_positionals, "relations")?;
        command = Some(Command::Relations {
            path,
            object,
            depth: relations_depth,
            direction: relations_direction,
            json: relations_json,
        });
    }
    if pending_context {
        let (path, query) = split_optional_path_and_query(&context_positionals, "context")?;
        command = Some(Command::Context {
            path,
            query,
            token_budget: context_budget,
            json: context_json,
        });
    }
    if pending_diff {
        command = Some(Command::Diff {
            path: diff_path,
            json: diff_json,
        });
    }
    if pending_impact {
        let (path, object) = split_optional_path_and_query(&impact_positionals, "impact")?;
        command = Some(Command::Impact {
            path,
            object,
            depth: impact_depth,
            json: impact_json,
        });
    }
    if pending_analyze {
        command = Some(Command::Analyze {
            path: analyze_path,
            scope: analyze_scope,
            json: analyze_json,
            format: analyze_format,
            output: analyze_output,
            include_suppressed: analyze_include_suppressed,
            suppressions: analyze_suppressions,
            fail_on: analyze_fail_on,
            fail_on_new: analyze_fail_on_new,
            baseline: analyze_baseline,
        });
    }
    if pending_install {
        command = Some(Command::Install {
            targets: parse_agent_kinds(install_target_raw.as_deref())
                .map_err(DbGraphError::invalid_argument)?,
            location: install_location,
            yes: install_yes,
            dry_run: install_dry_run,
            print_config: install_print_config,
        });
    }
    if pending_uninstall {
        command = Some(Command::Uninstall {
            targets: parse_agent_kinds(uninstall_target_raw.as_deref())
                .map_err(DbGraphError::invalid_argument)?,
            location: uninstall_location,
            dry_run: uninstall_dry_run,
        });
    }
    if pending_serve {
        if !serve_mcp {
            return Err(DbGraphError::invalid_argument(
                "`serve` requires `--mcp` in this release",
            ));
        }
        command = Some(Command::ServeMcp);
    }

    Ok(ParsedArgs {
        verbosity,
        command: command.unwrap_or(Command::Help),
    })
}

fn split_optional_path_and_query(
    positionals: &[String],
    command: &str,
) -> Result<(Option<PathBuf>, String)> {
    match positionals {
        [] => Err(DbGraphError::invalid_argument(format!(
            "`{command}` requires a query or object"
        ))),
        [query] => Ok((None, query.clone())),
        [first, rest @ ..] if looks_like_path(first) => {
            Ok((Some(PathBuf::from(first)), rest.join(" ")))
        }
        _ => Ok((None, positionals.join(" "))),
    }
}

fn looks_like_path(value: &str) -> bool {
    let path = Path::new(value);
    path.exists()
        || value == "."
        || value == ".."
        || value.starts_with("./")
        || value.starts_with("../")
        || value.starts_with(".\\")
        || value.starts_with("..\\")
        || value.contains('/')
        || value.contains('\\')
}

fn parse_depth(value: &str) -> Result<usize> {
    let depth = value
        .parse::<usize>()
        .map_err(|_| DbGraphError::invalid_argument("`--depth` requires 1 or 2"))?;
    if (1..=2).contains(&depth) {
        Ok(depth)
    } else {
        Err(DbGraphError::invalid_argument(
            "`--depth` supports only 1 or 2",
        ))
    }
}

fn parse_positive_usize(value: &str, message: &str) -> Result<usize> {
    let parsed = value
        .parse::<usize>()
        .map_err(|_| DbGraphError::invalid_argument(message))?;
    if parsed == 0 {
        Err(DbGraphError::invalid_argument(message))
    } else {
        Ok(parsed)
    }
}

fn parse_positive_u32(value: &str, message: &str) -> Result<u32> {
    let parsed = value
        .parse::<u32>()
        .map_err(|_| DbGraphError::invalid_argument(message))?;
    if parsed == 0 {
        Err(DbGraphError::invalid_argument(message))
    } else {
        Ok(parsed)
    }
}

fn parse_direction(value: &str) -> Result<Direction> {
    match value.to_ascii_lowercase().as_str() {
        "incoming" => Ok(Direction::Incoming),
        "outgoing" => Ok(Direction::Outgoing),
        "both" => Ok(Direction::Both),
        _ => Err(DbGraphError::invalid_argument(
            "`--direction` requires incoming, outgoing, or both",
        )),
    }
}

fn set_command(slot: &mut Option<Command>, next: Command) -> Result<()> {
    if slot.replace(next).is_some() {
        return Err(DbGraphError::invalid_argument(
            "only one command can be supplied",
        ));
    }
    Ok(())
}
