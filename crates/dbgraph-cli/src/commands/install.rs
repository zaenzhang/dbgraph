//! Agent install/uninstall CLI command implementations.

use std::path::Path;

use dbgraph_agent_config::{
    install_agent_config, render_mcp_config, uninstall_agent_config, AgentKind, AgentTarget,
};
use dbgraph_core::{DbGraphError, Result};

pub(crate) fn install_agents(
    targets: &[AgentKind],
    location: Option<&Path>,
    dry_run: bool,
    print_config: bool,
) -> Result<()> {
    if print_config {
        for target in targets {
            println!("# target: {target}");
            println!("{}", render_mcp_config(*target, "dbgraph").trim_end());
        }
        return Ok(());
    }

    for target in targets {
        let edit = install_agent_config(*target, location, "dbgraph", dry_run)
            .map_err(|source| DbGraphError::io(target.config_path(location), source))?;
        print_install_edit("install", &edit);
    }
    Ok(())
}

pub(crate) fn uninstall_agents(
    targets: &[AgentKind],
    location: Option<&Path>,
    dry_run: bool,
) -> Result<()> {
    for target in targets {
        let edit = uninstall_agent_config(*target, location, dry_run)
            .map_err(|source| DbGraphError::io(target.config_path(location), source))?;
        print_install_edit("uninstall", &edit);
    }
    Ok(())
}

fn print_install_edit(action: &str, edit: &dbgraph_agent_config::InstallEdit) {
    let mode = if edit.dry_run { "dry-run" } else { action };
    let status = if edit.changed { "changed" } else { "unchanged" };
    println!(
        "{mode} {target}: {status} {path}",
        target = edit.target,
        path = edit.path.display()
    );
    if let Some(backup) = &edit.backup_path {
        println!("backup: {}", backup.display());
    }
}
