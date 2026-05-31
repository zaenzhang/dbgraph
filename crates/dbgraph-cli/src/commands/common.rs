//! Shared CLI command helpers.

use std::env;
use std::path::{Path, PathBuf};

use dbgraph_core::model::DbSnapshot;
use dbgraph_core::project::ProjectContext;
use dbgraph_core::snapshot::SnapshotStore;
use dbgraph_core::{DbGraphError, Result};
use serde::Serialize;

pub(crate) fn discover_context(start: &Path) -> Result<ProjectContext> {
    Ok(ProjectContext::discover_from(start)?
        .unwrap_or_else(|| ProjectContext::from_project_root(start)))
}

pub(crate) fn path_or_current(path: Option<PathBuf>) -> Result<PathBuf> {
    path.map_or_else(
        || env::current_dir().map_err(|source| DbGraphError::io(".", source)),
        Ok,
    )
}

pub(crate) fn require_graph_index(context: &ProjectContext) -> Result<()> {
    if context.graph_db_path().is_file() {
        Ok(())
    } else {
        Err(DbGraphError::invalid_config(
            "graph index is missing; run `dbgraph snapshot` first",
        ))
    }
}

pub(crate) fn latest_snapshot(context: &ProjectContext) -> Result<DbSnapshot> {
    SnapshotStore::new(context).read_latest()?.ok_or_else(|| {
        DbGraphError::invalid_config("no snapshots found; run `dbgraph snapshot` first")
    })
}

pub(crate) fn print_json<T: Serialize>(value: &T) -> Result<()> {
    println!(
        "{}",
        serde_json::to_string_pretty(value).map_err(|source| DbGraphError::Internal {
            message: format!("failed to serialize JSON output: {source}"),
        })?
    );
    Ok(())
}

pub(crate) fn print_json_or<T: Serialize>(
    value: &T,
    json: bool,
    printer: impl FnOnce(&T),
) -> Result<()> {
    if json {
        print_json(value)
    } else {
        printer(value);
        Ok(())
    }
}
