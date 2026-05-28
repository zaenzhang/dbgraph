# DbGraph

DbGraph is a local-first database context engine for AI coding agents.

This repository currently contains the Rust workspace skeleton for the first
implementation task, `DBG-0001`.

## Workspace

```text
crates/
  dbgraph-cli/
  dbgraph-core/
  dbgraph-storage/
  dbgraph-provider/
  dbgraph-mcp/
  dbgraph-installer/
```

## Development

```bash
cargo fmt --all
cargo clippy --workspace --all-targets
cargo test --workspace
cargo run -p dbgraph-cli -- --version
```

The first milestone only provides the workspace boundary and a stable
`dbgraph --version` command. Database connections, MCP serving, installer
downloads, and graph indexing are intentionally left to later tasks.
