use std::env;
use std::io;
use std::process::ExitCode;

use dbgraph_core::{init_logging, version_string, DbGraphError, Result};
use dbgraph_mcp::run_stdio;
use tracing::debug;

mod args;

mod commands {
    pub(crate) mod analyze;
    pub(crate) mod benchmark_agent;
    pub(crate) mod common;
    pub(crate) mod doctor;
    pub(crate) mod init;
    pub(crate) mod inspect;
    pub(crate) mod install;
    pub(crate) mod snapshot;
}

use args::{parse_args, AgentBenchmarkFormat, AnalysisOutputFormat, Command};
use commands::analyze::{analyze_project_with_options, write_analysis_output, AnalysisCliOptions};
use commands::benchmark_agent::{
    benchmark_agent_project, benchmark_project, print_benchmark_report,
    write_agent_benchmark_output, AgentBenchmarkOptions,
};
use commands::common::{path_or_current, print_json, print_json_or};
use commands::doctor::{doctor_project, print_doctor_report, print_status, read_status};
use commands::init::{init_project_with_optional_snapshot, prompt_init_options, InitOptions};
use commands::inspect::{
    context_project, diff_project, impact_project, print_context_report, print_diff_report,
    print_impact_report, print_relations_report, print_search_report, print_table_report,
    print_validate_sql_report, read_sql_input, relations_project, search_project, table_project,
    validate_sql,
};
use commands::install::{install_agents, uninstall_agents};
use commands::snapshot::{
    print_snapshot_summary, print_sync_summary, run_snapshot, run_snapshot_with_options,
    sync_project, SnapshotCliOptions,
};

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
        Command::Doctor {
            path,
            json,
            check_db,
        } => {
            let root = path_or_current(path)?;
            let report = doctor_project(&root, check_db)?;
            print_json_or(&report, json, print_doctor_report)
        }
        Command::Snapshot {
            path,
            json,
            profile,
            max_rows_per_table,
            store_raw_samples,
        } => {
            let root = match path {
                Some(path) => path,
                None => env::current_dir().map_err(|source| DbGraphError::io(".", source))?,
            };
            let summary = run_snapshot_with_options(
                &root,
                SnapshotCliOptions {
                    profile,
                    max_rows_per_table,
                    store_raw_samples,
                },
            )?;
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
        Command::Sync { path, json } => {
            let root = path_or_current(path)?;
            let summary = sync_project(&root)?;
            print_json_or(&summary, json, print_sync_summary)
        }
        Command::Benchmark {
            tables,
            columns_per_table,
            json,
        } => {
            let report = benchmark_project(tables, columns_per_table)?;
            print_json_or(&report, json, print_benchmark_report)
        }
        Command::BenchmarkAgent {
            path,
            scenario,
            format,
            output,
        } => {
            let root = path_or_current(path)?;
            let report = benchmark_agent_project(&root, AgentBenchmarkOptions { scenario })?;
            write_agent_benchmark_output(&report, format, output.as_deref())
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
        Command::Search {
            path,
            query,
            kind,
            json,
        } => {
            let root = path_or_current(path)?;
            let report = search_project(&root, &query, kind.as_deref())?;
            print_json_or(&report, json, print_search_report)
        }
        Command::Table { path, table, json } => {
            let root = path_or_current(path)?;
            let report = table_project(&root, &table)?;
            print_json_or(&report, json, print_table_report)
        }
        Command::Relations {
            path,
            object,
            depth,
            direction,
            json,
        } => {
            let root = path_or_current(path)?;
            let report = relations_project(&root, &object, depth, direction)?;
            print_json_or(&report, json, print_relations_report)
        }
        Command::Context {
            path,
            query,
            token_budget,
            json,
        } => {
            let root = path_or_current(path)?;
            let report = context_project(&root, &query, token_budget)?;
            if json {
                print_json(&report)
            } else {
                print_context_report(&report);
                Ok(())
            }
        }
        Command::Diff { path, json } => {
            let root = path_or_current(path)?;
            let report = diff_project(&root)?;
            print_json_or(&report, json, print_diff_report)
        }
        Command::Impact {
            path,
            object,
            depth,
            json,
        } => {
            let root = path_or_current(path)?;
            let report = impact_project(&root, &object, depth)?;
            print_json_or(&report, json, print_impact_report)
        }
        Command::Analyze {
            path,
            scope,
            json,
            format,
            output,
            include_suppressed,
            suppressions,
            fail_on,
            fail_on_new,
            baseline,
        } => {
            let root = path_or_current(path)?;
            let report = analyze_project_with_options(
                &root,
                &AnalysisCliOptions {
                    scope,
                    include_suppressed,
                    suppressions,
                    fail_on,
                    fail_on_new,
                    baseline,
                },
            )?;
            let format = if json {
                AnalysisOutputFormat::Json
            } else {
                format
            };
            write_analysis_output(&report, format, output.as_deref())?;
            if report.gate.as_ref().is_some_and(|gate| !gate.passed) {
                return Err(DbGraphError::invalid_config(
                    report.gate.as_ref().map_or_else(
                        || "analysis gate failed".to_owned(),
                        |gate| gate.message.clone(),
                    ),
                ));
            }
            Ok(())
        }
        Command::Install {
            targets,
            location,
            yes: _,
            dry_run,
            print_config,
        } => install_agents(&targets, location.as_deref(), dry_run, print_config),
        Command::Uninstall {
            targets,
            location,
            dry_run,
        } => uninstall_agents(&targets, location.as_deref(), dry_run),
        Command::ServeMcp => run_stdio(io::stdin(), io::stdout(), io::stderr()),
    }
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
  dbgraph [OPTIONS] doctor [PATH] [--json] [--check-db]
  dbgraph [OPTIONS] snapshot [PATH] [--profile schema|stats|sample] [--max-rows-per-table N] [--store-raw-samples] [--json]
  dbgraph [OPTIONS] sync [PATH] [--json]
  dbgraph [OPTIONS] benchmark [--tables N] [--columns-per-table N] [--json]
  dbgraph [OPTIONS] benchmark-agent [PATH] --scenario teashop [--format text|json|markdown] [--output FILE]
  dbgraph [OPTIONS] validate-sql [PATH] (--sql SQL | --file FILE) [--dialect postgres|mysql|generic] [--json]
  dbgraph [OPTIONS] search [PATH] QUERY [--kind KIND] [--json]
  dbgraph [OPTIONS] table [PATH] TABLE [--json]
  dbgraph [OPTIONS] relations [PATH] OBJECT [--depth 1|2] [--direction incoming|outgoing|both] [--json]
  dbgraph [OPTIONS] context [PATH] QUERY [--tokens N] [--json]
  dbgraph [OPTIONS] diff [PATH] [--json]
  dbgraph [OPTIONS] impact [PATH] OBJECT [--depth 1|2] [--json]
  dbgraph [OPTIONS] analyze [PATH] [--scope all|risk|quality|performance] [--format text|json|markdown] [--output FILE] [--json] [--include-suppressed] [--suppressions FILE] [--fail-on SEVERITY] [--fail-on-new SEVERITY] [--baseline FILE]
  dbgraph [OPTIONS] install [--target codex,cursor,claude] [--location DIR] [--yes] [--dry-run] [--print-config]
  dbgraph [OPTIONS] uninstall [--target codex,cursor,claude] [--location DIR] [--dry-run]
  dbgraph [OPTIONS] serve --mcp
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
  -j, --json         Print command output as JSON

Commands:
  init       Initialize .dbgraph project state
  status     Show local project status
  doctor     Diagnose onboarding, config, snapshot, graph, PATH, and optional DB connectivity
  snapshot   Capture database schema into JSON and local SQLite index
  sync       Capture and compare schema hashes for incremental sync
  benchmark  Generate a synthetic schema benchmark report
  benchmark-agent Generate an offline agent value benchmark
  validate-sql Parse SQL and validate references against the local graph index
  search     Search schema and SQL graph objects
  table      Show table columns, constraints, profile, and relations
  relations  Traverse incoming, outgoing, explicit, and inferred relations
  context    Build compact read-only context for an AI database task
  diff       Compare the latest snapshot with the previous snapshot
  impact     Show downstream/upstream impact and risk notes for an object
  analyze    Report structured risk, quality, and performance findings
  install    Configure DbGraph MCP blocks for agent targets
  uninstall  Remove only DbGraph managed MCP blocks
  serve      Start the MCP stdio server"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::{Path, PathBuf};

    use dbgraph_agent_config::{AgentKind, AgentTarget};
    use dbgraph_core::config::{DatabaseConfig, DatabaseProviderKind, DbGraphConfig};
    use dbgraph_core::model::{
        ColumnMetadata, ColumnProfile, ConstraintMetadata, DbEdge, DbEdgeKind, DbObject,
        DbObjectKind, DbSnapshot, Evidence, IndexMetadata, TableProfile,
    };
    use dbgraph_core::profiling::ProfilingMode;
    use dbgraph_core::project::ProjectContext;
    use dbgraph_core::snapshot::SnapshotStore;
    use dbgraph_core::LogVerbosity;
    use dbgraph_graph::analysis::{AnalysisScope, FindingSeverity};
    use dbgraph_graph::relations::Direction;
    use dbgraph_storage::GraphRepository;

    use crate::args::ParsedArgs;
    use crate::commands::analyze::analyze_project;
    use crate::commands::doctor::DoctorStatus;
    use crate::commands::init::init_project;

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
                json: true,
                profile: None,
                max_rows_per_table: None,
                store_raw_samples: false,
            }
        );
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn parses_phase09_snapshot_sync_and_benchmark_commands() {
        let doctor = parse(&["doctor", "sample-project", "--json", "--check-db"])
            .expect("doctor args should parse");
        assert_eq!(
            doctor.command,
            Command::Doctor {
                path: Some(PathBuf::from("sample-project")),
                json: true,
                check_db: true,
            }
        );

        let snapshot = parse(&[
            "snapshot",
            "--profile",
            "sample",
            "--max-rows-per-table",
            "7",
            "--store-raw-samples",
            "--json",
        ])
        .expect("snapshot args should parse");
        assert_eq!(
            snapshot.command,
            Command::Snapshot {
                path: None,
                json: true,
                profile: Some(ProfilingMode::Sample),
                max_rows_per_table: Some(7),
                store_raw_samples: true,
            }
        );

        let sync = parse(&["sync", "sample-project", "--json"]).expect("sync args should parse");
        assert_eq!(
            sync.command,
            Command::Sync {
                path: Some(PathBuf::from("sample-project")),
                json: true,
            }
        );

        let benchmark = parse(&["benchmark", "--tables", "10000", "--columns-per-table", "2"])
            .expect("benchmark args should parse");
        assert_eq!(
            benchmark.command,
            Command::Benchmark {
                tables: 10_000,
                columns_per_table: 2,
                json: false,
            }
        );

        let analyze = parse(&["analyze", "sample-project", "--scope", "quality", "--json"])
            .expect("analyze args should parse");
        assert_eq!(
            analyze.command,
            Command::Analyze {
                path: Some(PathBuf::from("sample-project")),
                scope: AnalysisScope::Quality,
                json: true,
                format: AnalysisOutputFormat::Json,
                output: None,
                include_suppressed: false,
                suppressions: None,
                fail_on: None,
                fail_on_new: None,
                baseline: None,
            }
        );

        let markdown = parse(&[
            "analyze",
            "sample-project",
            "--scope",
            "all",
            "--format",
            "markdown",
            "--output",
            "report.md",
        ])
        .expect("markdown analyze args should parse");
        assert_eq!(
            markdown.command,
            Command::Analyze {
                path: Some(PathBuf::from("sample-project")),
                scope: AnalysisScope::All,
                json: false,
                format: AnalysisOutputFormat::Markdown,
                output: Some(PathBuf::from("report.md")),
                include_suppressed: false,
                suppressions: None,
                fail_on: None,
                fail_on_new: None,
                baseline: None,
            }
        );

        let gated = parse(&[
            "analyze",
            "--fail-on",
            "high",
            "--fail-on-new",
            "medium",
            "--baseline",
            "baseline.json",
            "--include-suppressed",
            "--suppressions",
            "suppressions.json",
        ])
        .expect("gated analyze args should parse");
        assert_eq!(
            gated.command,
            Command::Analyze {
                path: None,
                scope: AnalysisScope::All,
                json: false,
                format: AnalysisOutputFormat::Text,
                output: None,
                include_suppressed: true,
                suppressions: Some(PathBuf::from("suppressions.json")),
                fail_on: Some(FindingSeverity::High),
                fail_on_new: Some(FindingSeverity::Medium),
                baseline: Some(PathBuf::from("baseline.json")),
            }
        );

        let agent_benchmark = parse(&[
            "benchmark-agent",
            "sample-project",
            "--scenario",
            "teashop",
            "--format",
            "markdown",
            "--output",
            "benchmark.md",
        ])
        .expect("agent benchmark args should parse");
        assert_eq!(
            agent_benchmark.command,
            Command::BenchmarkAgent {
                path: Some(PathBuf::from("sample-project")),
                scenario: "teashop".to_owned(),
                format: AgentBenchmarkFormat::Markdown,
                output: Some(PathBuf::from("benchmark.md")),
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
    fn parses_serve_mcp_command() {
        let parsed = parse(&["serve", "--mcp"]).expect("serve mcp args should parse");

        assert_eq!(parsed.command, Command::ServeMcp);
    }

    #[test]
    fn parses_install_and_uninstall_agent_targets() {
        let install = parse(&[
            "install",
            "--target=codex,cursor,claude",
            "--location",
            "sandbox",
            "--yes",
            "--dry-run",
            "--print-config",
        ])
        .expect("install args should parse");
        assert_eq!(
            install.command,
            Command::Install {
                targets: vec![AgentKind::Codex, AgentKind::Cursor, AgentKind::Claude],
                location: Some(PathBuf::from("sandbox")),
                yes: true,
                dry_run: true,
                print_config: true,
            }
        );

        let uninstall = parse(&["uninstall", "--target", "codex", "--location=sandbox"])
            .expect("uninstall args should parse");
        assert_eq!(
            uninstall.command,
            Command::Uninstall {
                targets: vec![AgentKind::Codex],
                location: Some(PathBuf::from("sandbox")),
                dry_run: false,
            }
        );
    }

    #[test]
    fn install_and_uninstall_preserve_user_config() {
        let temp = TempProject::new();
        let target_path = AgentKind::Codex.config_path(Some(&temp.root));
        fs::create_dir_all(target_path.parent().expect("config has parent"))
            .expect("config dir should create");
        fs::write(&target_path, "{ \"user\": true }\n").expect("user config should write");

        install_agents(&[AgentKind::Codex], Some(&temp.root), false, false)
            .expect("install should write managed block");
        install_agents(&[AgentKind::Codex], Some(&temp.root), false, false)
            .expect("second install should be idempotent");
        let installed = fs::read_to_string(&target_path).expect("config should read");
        assert!(installed.contains("{ \"user\": true }"));
        assert_eq!(
            installed
                .matches(dbgraph_agent_config::DBGRAPH_MCP_SECTION_START)
                .count(),
            1
        );
        assert!(target_path.with_extension("dbgraph.bak").is_file());

        uninstall_agents(&[AgentKind::Codex], Some(&temp.root), false)
            .expect("uninstall should remove managed block");
        let removed = fs::read_to_string(&target_path).expect("config should read");
        assert!(removed.contains("{ \"user\": true }"));
        assert!(!removed.contains(dbgraph_agent_config::DBGRAPH_MCP_SECTION_START));
    }

    #[test]
    fn parses_phase05_commands_without_quoted_query() {
        let search = parse(&["search", "refund", "payment", "--kind", "table", "--json"])
            .expect("search args should parse");
        assert_eq!(
            search.command,
            Command::Search {
                path: None,
                query: "refund payment".to_owned(),
                kind: Some("table".to_owned()),
                json: true
            }
        );

        let relations = parse(&[
            "relations",
            "public.payments",
            "--depth",
            "2",
            "--direction",
            "incoming",
        ])
        .expect("relations args should parse");
        assert_eq!(
            relations.command,
            Command::Relations {
                path: None,
                object: "public.payments".to_owned(),
                depth: 2,
                direction: Direction::Incoming,
                json: false
            }
        );

        let context = parse(&[
            "context", "refund", "payment", "order", "--tokens", "120", "--json",
        ])
        .expect("context args should parse");
        assert_eq!(
            context.command,
            Command::Context {
                path: None,
                query: "refund payment order".to_owned(),
                token_budget: 120,
                json: true
            }
        );

        let impact = parse(&["impact", "public.orders.status", "--depth", "1"])
            .expect("impact args should parse");
        assert_eq!(
            impact.command,
            Command::Impact {
                path: None,
                object: "public.orders.status".to_owned(),
                depth: 1,
                json: false
            }
        );
    }

    #[test]
    fn parses_phase05_commands_with_explicit_path() {
        let temp = TempProject::new();
        fs::create_dir_all(&temp.root).expect("temp root should exist");

        let parsed = parse(&[
            "search",
            temp.root.to_str().expect("path should be utf8"),
            "payment",
            "orders",
        ])
        .expect("search args should parse");

        assert_eq!(
            parsed.command,
            Command::Search {
                path: Some(temp.root.clone()),
                query: "payment orders".to_owned(),
                kind: None,
                json: false
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
    fn search_requires_graph_index_and_returns_stable_matches() {
        let temp = TempProject::new();
        init_project(&temp.root, false, &InitOptions::default()).expect("init should succeed");
        let snapshot = sample_phase05_snapshot("s1", 1);
        write_latest_snapshot_without_index(&temp.root, &snapshot);

        let err = search_project(&temp.root, "payment", Some("table"))
            .expect_err("search should require graph index");
        assert!(err.to_string().contains("dbgraph snapshot"));

        write_latest_snapshot_with_index(&temp.root, &snapshot);
        let report = search_project(&temp.root, "payment", Some("table"))
            .expect("search should use local snapshot");

        assert_eq!(report.results[0].full_name, "public.payments");
        assert!(report.results.iter().all(|result| result.kind == "table"));
    }

    #[test]
    fn table_reports_columns_constraints_profiles_and_relations() {
        let temp = TempProject::new();
        init_project(&temp.root, false, &InitOptions::default()).expect("init should succeed");
        let snapshot = sample_phase05_snapshot("s1", 1);
        write_latest_snapshot_with_index(&temp.root, &snapshot);

        let report = table_project(&temp.root, "payments").expect("table should resolve");

        assert_eq!(report.table, "public.payments");
        assert!(report.columns.iter().any(|column| column.name == "order_id"
            && column.data_type.as_deref() == Some("bigint")
            && column.nullable == Some(false)));
        assert!(report
            .constraints
            .iter()
            .any(|object| object.kind == "foreign_key"));
        assert!(report
            .indexes
            .iter()
            .any(|object| object.full_name == "public.idx_payments_order_id"));
        assert_eq!(
            report
                .profile
                .as_ref()
                .and_then(|profile| profile.row_estimate),
            Some(12_500)
        );
        assert!(report
            .incoming_relations
            .iter()
            .any(|edge| edge.from == "public.refunds"));
        assert!(report
            .outgoing_relations
            .iter()
            .any(|edge| edge.to == "public.orders"));

        let missing = table_project(&temp.root, "paymnts").expect("missing table should suggest");
        assert!(missing.suggestions.contains(&"public.payments".to_owned()));
    }

    #[test]
    fn relations_context_diff_and_impact_reports_are_built_from_snapshots() {
        let temp = TempProject::new();
        init_project(&temp.root, false, &InitOptions::default()).expect("init should succeed");
        let previous = sample_phase05_snapshot("s1", 1);
        let latest = sample_phase05_snapshot("s2", 2);
        write_snapshot_only(&temp.root, &previous);
        write_latest_snapshot_with_index(&temp.root, &latest);

        let relations = relations_project(&temp.root, "public.orders", 2, Direction::Incoming)
            .expect("relations should resolve");
        assert!(relations.paths.iter().any(|path| {
            path.objects
                .first()
                .is_some_and(|name| name == "public.orders")
                && path
                    .objects
                    .last()
                    .is_some_and(|name| name == "public.payments")
                && path
                    .edges
                    .iter()
                    .any(|edge| edge.from == "public.payments" && edge.to == "public.orders")
        }));

        let context =
            context_project(&temp.root, "refund payment order", 120).expect("context should build");
        let names = context
            .objects
            .iter()
            .map(|object| object.full_name.as_str())
            .collect::<Vec<_>>();
        assert!(names.contains(&"public.refunds"));
        assert!(names.contains(&"public.payments"));
        assert!(context
            .risks
            .iter()
            .any(|risk| risk.to_ascii_lowercase().contains("read-only")));

        let diff = diff_project(&temp.root).expect("diff should compare latest and previous");
        assert!(diff
            .changes
            .iter()
            .any(|change| change.full_name == "public.refunds.status"));

        let impact =
            impact_project(&temp.root, "public.orders.status", 2).expect("impact should resolve");
        assert!(impact
            .items
            .iter()
            .any(|item| item.full_name == "public.active_orders"));
        assert!(impact
            .risks
            .iter()
            .any(|risk| risk.message.to_ascii_lowercase().contains("status")));
    }

    #[test]
    fn analyze_reports_quality_findings_from_latest_snapshot() {
        let temp = TempProject::new();
        init_project(&temp.root, false, &InitOptions::default()).expect("init should succeed");
        let snapshot = sample_phase05_snapshot("s1", 1);
        write_latest_snapshot_with_index(&temp.root, &snapshot);

        let report = analyze_project(&temp.root, AnalysisScope::Quality)
            .expect("analysis should use latest snapshot");

        assert!(report
            .findings
            .iter()
            .any(|finding| finding.rule_id == "quality.missing_primary_key"
                && finding.object == "public.orders"));
        assert!(report
            .findings
            .iter()
            .all(|finding| finding.scope == AnalysisScope::Quality));
    }

    #[test]
    fn analyze_suppresses_exact_active_findings_and_marks_gate() {
        let temp = TempProject::new();
        init_project(&temp.root, false, &InitOptions::default()).expect("init should succeed");
        let snapshot = sample_phase05_snapshot("s1", 1);
        write_latest_snapshot_with_index(&temp.root, &snapshot);
        let suppression_path = ProjectContext::from_project_root(&temp.root)
            .dbgraph_dir()
            .join("suppressions.json");
        fs::write(
            &suppression_path,
            r#"{
              "version": 1,
              "suppressions": [
                {
                  "ruleId": "quality.missing_primary_key",
                  "object": "public.orders",
                  "reason": "legacy fixture",
                  "owner": "data-platform",
                  "expiresAt": "2999-12-31"
                },
                {
                  "ruleId": "quality.missing_primary_key",
                  "object": "public.payments",
                  "reason": "expired fixture",
                  "owner": "data-platform",
                  "expiresAt": "1970-01-01"
                }
              ]
            }"#,
        )
        .expect("suppression file should write");

        let report = analyze_project_with_options(
            &temp.root,
            &AnalysisCliOptions {
                scope: AnalysisScope::Quality,
                include_suppressed: false,
                suppressions: Some(suppression_path),
                fail_on: Some(FindingSeverity::Medium),
                fail_on_new: None,
                baseline: None,
            },
        )
        .expect("analysis should run");

        assert!(!report.findings.iter().any(|finding| {
            finding.rule_id == "quality.missing_primary_key" && finding.object == "public.orders"
        }));
        assert!(report.suppressed_findings.iter().any(|finding| {
            finding.rule_id == "quality.missing_primary_key" && finding.object == "public.orders"
        }));
        assert!(report.findings.iter().any(|finding| {
            finding.rule_id == "quality.missing_primary_key" && finding.object == "public.payments"
        }));
        assert!(report
            .gate
            .as_ref()
            .is_some_and(|gate| !gate.passed && gate.threshold.as_deref() == Some("medium")));
    }

    #[test]
    fn doctor_reports_missing_and_initialized_project_state() {
        let temp = TempProject::new();
        fs::create_dir_all(&temp.root).expect("temp root should exist");

        let missing = doctor_project(&temp.root, false).expect("doctor should not fail hard");
        assert_eq!(missing.status, DoctorStatus::Error);
        assert!(missing
            .suggested_next_commands
            .contains(&"dbgraph init -i --yes".to_owned()));

        init_project(&temp.root, false, &InitOptions::default()).expect("init should succeed");
        let initialized = doctor_project(&temp.root, false).expect("doctor should inspect project");
        assert!(initialized
            .checks
            .iter()
            .any(|check| check.id == "config" && check.status == DoctorStatus::Ok));
        assert!(initialized
            .checks
            .iter()
            .any(|check| check.id == "graph_index" && check.status == DoctorStatus::Warning));
    }

    #[test]
    fn benchmark_agent_computes_metrics_from_snapshot_and_project_files() {
        let temp = TempProject::new();
        init_project(&temp.root, false, &InitOptions::default()).expect("init should succeed");
        fs::create_dir_all(temp.root.join("sql")).expect("sql dir should exist");
        fs::write(
            temp.root.join("schema.sql"),
            "CREATE TABLE customers (id bigint primary key, email text);",
        )
        .expect("schema should write");
        fs::write(
            temp.root.join("sql").join("orders.sql"),
            "SELECT o.status, c.email FROM orders o JOIN customers c ON c.id = o.customer_id;",
        )
        .expect("sql should write");
        let snapshot = sample_phase05_snapshot("s1", 1);
        write_latest_snapshot_with_index(&temp.root, &snapshot);

        let report = benchmark_agent_project(
            &temp.root,
            AgentBenchmarkOptions {
                scenario: "teashop".to_owned(),
            },
        )
        .expect("benchmark should run");

        assert_eq!(report.scenario, "teashop");
        assert!(report.summary.dbgraph_estimated_tokens > 0);
        assert!(report.summary.baseline_estimated_tokens > 0);
        assert!(report.cases.iter().any(|case| case
            .expected_objects
            .contains(&"public.orders.status".to_owned())));
        assert!(report.summary.evidence_recall_delta >= 0.0);
    }

    #[test]
    fn analyze_writes_markdown_report_to_output_path() {
        let temp = TempProject::new();
        init_project(&temp.root, false, &InitOptions::default()).expect("init should succeed");
        let snapshot = sample_phase05_snapshot("s1", 1);
        write_latest_snapshot_with_index(&temp.root, &snapshot);
        let report_path = temp.root.join("analysis.md");

        let report = analyze_project(&temp.root, AnalysisScope::All).expect("analysis should run");
        write_analysis_output(
            &report,
            AnalysisOutputFormat::Markdown,
            Some(report_path.as_path()),
        )
        .expect("markdown report should write");

        let markdown = fs::read_to_string(report_path).expect("markdown report should be readable");
        assert!(markdown.contains("# DbGraph Analysis Report"));
        assert!(markdown.contains("Data Integrity & Schema Quality"));
        assert!(markdown.contains("Suggested fix"));
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
    fn snapshot_supports_sqlite_provider_without_external_service() {
        let temp = TempProject::new();
        fs::create_dir_all(&temp.root).expect("temp root should exist");
        let sqlite_path = temp.root.join("business.sqlite");
        let connection = rusqlite::Connection::open(&sqlite_path).expect("sqlite should open");
        connection
            .execute_batch(
                "
                CREATE TABLE users (
                    id INTEGER PRIMARY KEY,
                    email TEXT NOT NULL
                );
                INSERT INTO users (id, email) VALUES (1, 'a@example.test');
                ",
            )
            .expect("sqlite fixture should write");
        let context = ProjectContext::from_project_root(&temp.root);
        let config = DbGraphConfig {
            database: DatabaseConfig {
                provider: DatabaseProviderKind::Sqlite.to_string(),
                connection_env: None,
                connection_string: Some(sqlite_path.display().to_string()),
            },
            ..DbGraphConfig::default()
        };
        config.save(&context).expect("sqlite config should save");

        let summary = run_snapshot(&temp.root).expect("sqlite snapshot should run");

        assert_eq!(summary.provider, "sqlite");
        assert_eq!(summary.table_count, 1);
        assert_eq!(summary.column_count, 2);
        assert!(summary.snapshot_path.is_file());
        assert!(summary.graph_db_path.is_file());
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

    fn write_latest_snapshot_without_index(root: &Path, snapshot: &DbSnapshot) {
        write_snapshot_only(root, snapshot);
    }

    fn write_latest_snapshot_with_index(root: &Path, snapshot: &DbSnapshot) {
        let context = ProjectContext::from_project_root(root);
        SnapshotStore::new(&context)
            .write_snapshot(snapshot, true)
            .expect("snapshot should write");
        let mut repo = GraphRepository::open(context.graph_db_path()).expect("repo should open");
        repo.rebuild_snapshot(snapshot)
            .expect("graph index should write");
    }

    fn write_snapshot_only(root: &Path, snapshot: &DbSnapshot) {
        let context = ProjectContext::from_project_root(root);
        SnapshotStore::new(&context)
            .write_snapshot(snapshot, true)
            .expect("snapshot should write");
    }

    #[allow(clippy::too_many_lines)]
    fn sample_phase05_snapshot(id: &str, created_at_unix_ms: u64) -> DbSnapshot {
        let mut snapshot = DbSnapshot::new(id, "postgres", "app", created_at_unix_ms);
        snapshot
            .objects
            .push(table("table:orders", "public.orders", "orders"));
        snapshot
            .objects
            .push(table("table:payments", "public.payments", "payments"));
        snapshot
            .objects
            .push(table("table:refunds", "public.refunds", "refunds"));
        snapshot.objects.push(column(
            "column:orders.status",
            "public.orders.status",
            "orders",
            "status",
            "text",
            false,
            Some("'pending'"),
        ));
        snapshot.objects.push(column(
            "column:payments.order_id",
            "public.payments.order_id",
            "payments",
            "order_id",
            "bigint",
            false,
            None,
        ));
        snapshot.objects.push(column(
            "column:refunds.payment_id",
            "public.refunds.payment_id",
            "refunds",
            "payment_id",
            "bigint",
            false,
            None,
        ));
        if id == "s2" {
            snapshot.objects.push(column(
                "column:refunds.status",
                "public.refunds.status",
                "refunds",
                "status",
                "text",
                false,
                Some("'open'"),
            ));
        }
        snapshot.objects.push(foreign_key(
            "fk:payments.orders",
            "public.payments_order_id_fkey",
            "payments",
            "public.orders",
        ));
        snapshot.objects.push(index(
            "index:payments.order_id",
            "public.idx_payments_order_id",
            "payments",
        ));
        snapshot.objects.push(view(
            "view:active_orders",
            "public.active_orders",
            "active order status view",
        ));
        snapshot.edges.push(edge(
            "edge:payments.orders",
            DbEdgeKind::References,
            "table:payments",
            "table:orders",
            1.0,
            "foreign key payments.order_id",
        ));
        snapshot.edges.push(edge(
            "edge:refunds.payments",
            DbEdgeKind::InferredReference,
            "table:refunds",
            "table:payments",
            0.74,
            "naming rule refunds.payment_id",
        ));
        snapshot.edges.push(edge(
            "edge:view.orders",
            DbEdgeKind::UsedByView,
            "view:active_orders",
            "table:orders",
            1.0,
            "view definition",
        ));
        snapshot.edges.push(edge(
            "edge:view.status",
            DbEdgeKind::FiltersBy,
            "view:active_orders",
            "column:orders.status",
            1.0,
            "where status = 'active'",
        ));
        snapshot.table_profiles.push(TableProfile {
            object_id: "table:payments".to_owned(),
            row_estimate: Some(12_500),
            row_count_kind: Some("estimate".to_owned()),
            size_bytes: Some(524_288),
            profile: std::collections::BTreeMap::new(),
        });
        snapshot.column_profiles.push(ColumnProfile {
            object_id: "column:payments.order_id".to_owned(),
            data_type_family: Some("integer".to_owned()),
            null_fraction: Some(0.0),
            distinct_estimate: Some(11_000.0),
            pii_score: Some(0.0),
            profile: std::collections::BTreeMap::new(),
        });
        snapshot
    }

    fn table(id: &str, full_name: &str, table_name: &str) -> DbObject {
        let mut object = DbObject::new(id, DbObjectKind::Table, full_name);
        object.schema_name = Some("public".to_owned());
        object.table_name = Some(table_name.to_owned());
        object.metadata.insert(
            "comment".to_owned(),
            serde_json::Value::String(format!("{table_name} records")),
        );
        object
    }

    fn column(
        id: &str,
        full_name: &str,
        table_name: &str,
        column_name: &str,
        data_type: &str,
        nullable: bool,
        default: Option<&str>,
    ) -> DbObject {
        let mut object = DbObject::new(id, DbObjectKind::Column, full_name);
        object.schema_name = Some("public".to_owned());
        object.table_name = Some(table_name.to_owned());
        object.column_name = Some(column_name.to_owned());
        object.column = Some(ColumnMetadata {
            data_type: Some(data_type.to_owned()),
            data_type_family: Some(data_type.to_owned()),
            nullable: Some(nullable),
            default: default.map(ToOwned::to_owned),
            comment: None,
        });
        object
    }

    fn foreign_key(
        id: &str,
        full_name: &str,
        table_name: &str,
        referenced_table: &str,
    ) -> DbObject {
        let mut object = DbObject::new(id, DbObjectKind::ForeignKey, full_name);
        object.schema_name = Some("public".to_owned());
        object.table_name = Some(table_name.to_owned());
        object.constraint = Some(ConstraintMetadata {
            columns: vec!["order_id".to_owned()],
            referenced_table: Some(referenced_table.to_owned()),
            referenced_columns: vec!["id".to_owned()],
        });
        object
    }

    fn index(id: &str, full_name: &str, table_name: &str) -> DbObject {
        let mut object = DbObject::new(id, DbObjectKind::Index, full_name);
        object.schema_name = Some("public".to_owned());
        object.table_name = Some(table_name.to_owned());
        object.index = Some(IndexMetadata {
            unique: Some(false),
            columns: vec!["order_id".to_owned()],
            expression: None,
        });
        object
    }

    fn view(id: &str, full_name: &str, comment: &str) -> DbObject {
        let mut object = DbObject::new(id, DbObjectKind::View, full_name);
        object.schema_name = Some("public".to_owned());
        object.metadata.insert(
            "comment".to_owned(),
            serde_json::Value::String(comment.to_owned()),
        );
        object
    }

    fn edge(
        id: &str,
        kind: DbEdgeKind,
        from: &str,
        to: &str,
        confidence: f64,
        detail: &str,
    ) -> DbEdge {
        let mut edge = DbEdge::explicit(id, kind, from, to);
        edge.confidence = confidence;
        edge.evidence.push(Evidence {
            source: "test".to_owned(),
            detail: detail.to_owned(),
        });
        edge
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
