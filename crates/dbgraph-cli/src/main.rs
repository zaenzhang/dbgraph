use std::env;
use std::process::ExitCode;

use dbgraph_core::{init_logging, version_string, DbGraphError, LogVerbosity, Result};
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
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ParsedArgs {
    verbosity: LogVerbosity,
    command: Command,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Command {
    Version,
    Help,
}

fn parse_args(args: impl IntoIterator<Item = String>) -> Result<ParsedArgs> {
    let mut verbosity = LogVerbosity::Normal;
    let mut command = None;

    for arg in args {
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
            _ => {
                return Err(DbGraphError::invalid_argument(format!(
                    "unknown command or option `{arg}`"
                )));
            }
        }
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
  dbgraph [OPTIONS] --version
  dbgraph [OPTIONS] --help

Options:
  -v, --verbose  Show debug diagnostics
  -q, --quiet    Show errors only
  -V, --version  Print version
  -h, --help     Print help

Commands and database features will be added by later tasks."
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
}
