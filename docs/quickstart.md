# DbGraph Quickstart

Get DbGraph running in a project in seconds. The recommended path is a small installer that downloads a prebuilt CLI binary, puts `dbgraph` on your `PATH`, initializes local `.dbgraph/` state, and writes MCP configuration for your coding agent.

For the complete workflow, see [usage.md](usage.md).
For project config fields, see [configuration.md](configuration.md).
中文完整说明见 [usage.zh-CN.md](usage.zh-CN.md)。

## Recommended: Install The CLI

No Node.js or local Rust toolchain is required.

```bash
# macOS / Linux
curl -fsSL https://raw.githubusercontent.com/zaenzhang/dbgraph/master/install.sh | sh
```

```powershell
# Windows PowerShell
irm https://raw.githubusercontent.com/zaenzhang/dbgraph/master/install.ps1 | iex
```

## Initialize A Project

```bash
cd your-project
dbgraph init -i --yes
dbgraph install --target codex --yes
```

Use `--target cursor`, `--target claude`, `--target gemini`, or `--target opencode` for other agent configs.

That is the main non-invasive integration: DbGraph stores project state under `.dbgraph/`, and the agent config points to the stable `dbgraph serve --mcp` command.

## Optional: Try With npx

Use npx only for quick experiments or CI smoke checks. It is not the preferred long-running MCP command because agent MCP config should point to a stable executable on `PATH`.

```bash
npx github:zaenzhang/dbgraph --version
npx github:zaenzhang/dbgraph init -i --yes
```

After the npm package is published, the command becomes:

```bash
npx @dbgraph/cli --version
npm i -g @dbgraph/cli
```

The Node wrapper downloads the matching GitHub Release asset, verifies its SHA256 checksum, caches the binary locally, and forwards your CLI arguments to `dbgraph`.

For agent MCP configuration, install `dbgraph` on your `PATH` first with the shell installer or a future package-manager install, then run:

```bash
dbgraph install --target codex --yes
```

## Configure A Database

The interactive initializer writes `.dbgraph/dbgraph.config.json`. PostgreSQL uses `DATABASE_URL` by default:

```bash
export DATABASE_URL="postgres://postgres:postgres@localhost:55432/teashop"
dbgraph snapshot --profile stats
```

PowerShell:

```powershell
$env:DATABASE_URL="postgres://postgres:postgres@localhost:55432/teashop"
dbgraph snapshot --profile stats
```

SQLite works without an external service. Set the provider and connection string in `.dbgraph/dbgraph.config.json`:

```json
{
  "version": 1,
  "database": {
    "provider": "sqlite",
    "connectionEnv": null,
    "connectionString": "C:/path/to/app.sqlite"
  },
  "snapshot": {
    "prettyJson": true,
    "profilingMode": "schema",
    "maxRowsPerTable": 20,
    "sampleRows": false
  },
  "security": {
    "storeRawData": false,
    "storeRawSamples": false,
    "maskPii": true,
    "customSensitiveTerms": []
  },
  "mcp": {
    "enabled": true,
    "maxResponseChars": 15000
  }
}
```

## Daily Commands

```bash
dbgraph status
dbgraph search customer --kind table
dbgraph table public.orders
dbgraph relations public.orders --depth 2
dbgraph context "refund payment order" --tokens 800
dbgraph validate-sql --sql "select * from orders"
dbgraph diff
dbgraph impact public.orders.status
dbgraph analyze --scope all --format markdown --output dbgraph-analysis.md
```

## What The Agent Gets

When a project has `.dbgraph/`, your configured agent can use DbGraph MCP tools for database-structure questions:

- `dbgraph_search`
- `dbgraph_table`
- `dbgraph_context`
- `dbgraph_relations`
- `dbgraph_impact`
- `dbgraph_analyze`
- `dbgraph_diff`
- `dbgraph_validate_sql`

DbGraph remains read-only during analysis: `validate-sql` does not execute SQL, and `analyze` works from the local snapshot and graph index.
