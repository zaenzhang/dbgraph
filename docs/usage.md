# DbGraph Usage Guide

DbGraph is a local-first database context tool. The normal workflow is:

1. initialize a project
2. configure a database provider
3. capture a snapshot
4. use the local graph for search, validation, impact analysis, and review reports

DbGraph stores schema metadata, SQL artifacts, graph edges, and profile summaries. It does not execute SQL during validation, and it does not store business row data by default.

For the full `.dbgraph/dbgraph.config.json` reference, see [configuration.md](configuration.md).

## Install Or Run

Recommended path. No Node.js or local Rust toolchain is required:

```bash
# macOS / Linux
curl -fsSL https://raw.githubusercontent.com/zaenzhang/dbgraph/master/install.sh | sh
```

```powershell
# Windows PowerShell
irm https://raw.githubusercontent.com/zaenzhang/dbgraph/master/install.ps1 | iex
```

Optional npx path for quick experiments:

```bash
npx github:zaenzhang/dbgraph --version
npx github:zaenzhang/dbgraph init -i --yes
```

After the npm package is published:

```bash
npx @dbgraph/cli --version
npm i -g @dbgraph/cli
```

When developing from this repository, prefix CLI commands with Cargo:

```powershell
cargo run -p dbgraph-cli -- --version
cargo run -p dbgraph-cli -- init -i --yes
```

If `dbgraph` is already installed on your `PATH`, use the shorter form:

```powershell
dbgraph --version
dbgraph init -i --yes
```

## Initialize A Project

Run this from the application or database-project directory you want DbGraph to index:

```powershell
dbgraph init -i --yes
```

Configure an agent MCP entry without printing or copying JSON manually:

```powershell
dbgraph install --target codex --yes
dbgraph install --target cursor --yes
dbgraph install --target claude --yes
```

Agent MCP config expects `dbgraph` to be available on `PATH`, so use the shell installer or a future package-manager install before running `dbgraph install`.

This creates:

- `.dbgraph/dbgraph.config.json`
- `.dbgraph/snapshots/`
- `.dbgraph/instructions/`
- `.dbgraph/dbgraph.db` after the first successful snapshot

Check project state:

```powershell
dbgraph status
dbgraph status --json
dbgraph doctor
dbgraph doctor --json --check-db
```

`doctor` is CLI-only in this release so it can inspect local `PATH` and optional provider connectivity without expanding the MCP surface.

## Configure A Database

DbGraph reads configuration from `.dbgraph/dbgraph.config.json`.

The main configuration sections are:

- `database`: provider and connection source.
- `snapshot`: JSON formatting, profiling depth, and sample limits.
- `security`: raw-data storage, sample masking, and custom sensitive terms.
- `mcp`: MCP enablement and response budget.
- `dataAccess`: explicit table/column allowlist for business-row sampling.

### PostgreSQL

The default interactive config uses `DATABASE_URL`.

```powershell
$env:DATABASE_URL="postgres://postgres:postgres@localhost:55432/teashop"
dbgraph snapshot --profile stats
```

### SQLite

SQLite does not require an external service. Set the provider and connection string in `.dbgraph/dbgraph.config.json`:

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

MySQL and SQL Server are registered as providers but are intentionally skipped in this build until local/container fixtures are available.

## Capture A Snapshot

```powershell
dbgraph snapshot
dbgraph snapshot --profile schema
dbgraph snapshot --profile stats
dbgraph snapshot --profile sample --max-rows-per-table 20
```

Profile modes:

- `schema`: schema-only metadata; safest default.
- `stats`: provider/catalog statistics such as row estimates.
- `sample`: enables allowlisted row sampling only when `dataAccess` also permits it; masked by default.

Snapshot output is written under `.dbgraph/snapshots/`, and the local graph index is rebuilt into `.dbgraph/dbgraph.db`.

A snapshot becomes the latest version only after its graph index and SQL artifacts
have been stored successfully. A failed capture leaves the previous published
snapshot as the latest version.

Use `dbgraph sync` to refresh a project. When the schema is unchanged and the
index is healthy, sync skips rebuilding. If the index is missing, sync rebuilds
it from the published snapshot instead of reporting a successful skip. A repair
failure is reported as an error; it does not create a schema change.
Sync still captures current database metadata before deciding whether the schema
is unchanged, so the configured provider must be reachable. It is not an offline
index-repair command.

### Allowlisted Data Profiling

By default, DbGraph does not read business row values. To analyze bounded samples for business-rule signals, enable sample profiling and allowlist exact columns:

This is also the recommended way to filter database exposure for AI agents:
tables can remain `schemaOnly` for structure and lineage, while only approved
tables/columns use `sample`. The generated graph context and analysis reports
reuse those authorized summaries and do not query additional row values.

```json
{
  "snapshot": {
    "profilingMode": "sample",
    "maxRowsPerTable": 50,
    "sampleRows": true
  },
  "dataAccess": {
    "defaultMode": "schemaOnly",
    "tables": [
      {
        "pattern": "public.orders",
        "mode": "sample",
        "columns": ["status", "created_at"],
        "where": "created_at >= now() - interval '30 days'",
        "limit": 50,
        "storeRawValues": false
      },
      {
        "pattern": "public.payments",
        "mode": "schemaOnly"
      }
    ]
  }
}
```

Then run:

```powershell
dbgraph snapshot --profile sample
dbgraph analyze --format markdown
```

`analyze` adds a `Data Profiling & Business Rules` section when allowlisted sample summaries reveal enum-like unconstrained values, high null rates, negative metric-like values, or unstable identifier formats.

### Semantic Metadata

Use `.dbgraph/semantics.json` to add business meaning that is not available from schema metadata:

```json
{
  "version": 1,
  "objects": [
    {
      "object": "public.orders.status",
      "description": "Order lifecycle state",
      "owner": "commerce",
      "allowedValues": ["pending", "paid", "shipped", "cancelled"],
      "deprecated": false,
      "certified": true
    }
  ]
}
```

Then refresh the snapshot:

```powershell
dbgraph snapshot
dbgraph context "order status"
dbgraph analyze --scope quality
```

`context` and MCP context responses include semantic metadata. If a SQL artifact references an object marked `deprecated: true`, `analyze` reports `quality.deprecated_object_used`.

## SQL Artifacts

During snapshot, DbGraph scans SQL files under these directories by default:

- `migrations/`
- `sql/`
- `db/`

It ignores noisy directories such as `node_modules/`, `target/`, `bin/`, and `obj/`.

SQL artifacts become query objects in the graph. DbGraph extracts read/write/filter/join dependencies where supported.

Each source file has its own query identity. Files containing the same SQL are
retained with their source paths; the SQL fingerprint groups equivalent workload
signals without replacing either source. Renaming or deleting a source is visible
as a query-object change in the next snapshot diff.

## Daily CLI Commands

Search graph objects:

```powershell
dbgraph search customer
dbgraph search orders --kind table
dbgraph search email --kind column --json
```

Search preserves Unicode words and short identifiers. Chinese terms can match
object names or semantic descriptions; this is lexical matching, not automatic
translation. A nonempty query with no match returns no results rather than an
unfiltered object list.

CLI and MCP share the same search report. `snapshot` contains `id`, `provider`,
`databaseName`, and `createdAtUnixMs`; `liveState: "not_checked"` means the capture
time does not establish that today's database is unchanged. SQL results retain
the existing `sourcePath` from the captured object's metadata.

`diagnostics.matchMode: "lexical_or"` means any extracted term can match a name,
full name, or metadata. `matchedTerms` and `unmatchedTerms` describe evidence
across the filtered candidates, including matches omitted by the result limit;
`uncheckedTerms` applies when there were no candidates to examine. Empty term
lists are omitted. Status distinguishes `matched`, `empty_snapshot`,
`no_search_terms`, `no_candidates`, and `no_term_match`. A partial lexical match
does not mean the whole task is answered; no match does not prove a business
concept absent. `filters` records the applied kind/schema/name filters, and
`diagnostics.coverage` counts snapshot objects, candidates, matches, returned
results, and `omittedByLimit`. Filter-only queries deliberately browse a subset;
DbGraph does not automatically loosen a filter or rewrite an unmatched query.

Inspect a table:

```powershell
dbgraph table public.orders
dbgraph table orders --json
```

Traverse relationships:

```powershell
dbgraph relations public.orders --depth 2
dbgraph relations public.orders --direction incoming
```

Build compact AI context:

```powershell
dbgraph context "refund payment order" --tokens 800
dbgraph context "which tables are touched by order status changes" --json
```

Context reuses those lexical diagnostics for its seeds, then expands one graph
hop. Objects identify their snapshot-local `id` and `origin`: `lexical_match`,
`filter_match`, or `graph_neighbor`. A neighbor need not match the query or its
filters; filters constrain seeds. Context `coverage` counts `seedObjects` and
`neighborObjects`, plus `omittedByLimit`, `omittedByBudget`, `omittedRelations`,
and `omittedSuggestions`. Relation omissions count edges between returned
objects, not every edge in the snapshot.

`--tokens` budgets the complete pretty-printed context JSON, including provenance,
diagnostics, semantic metadata, relationships, concrete suggestions, omission
counts, and formatting. `estimatedTokens` rounds
the JSON's UTF-8 byte length divided by four up to a whole number; this is a
rough estimate, not an LLM tokenizer or a billing measurement. DbGraph omits
content to fit the budget and reports a clear error if the budget cannot hold a
usable JSON response; it never cuts JSON in the middle. Increase the budget or
narrow the query when more detail is needed.

Validate SQL without executing it:

```powershell
dbgraph validate-sql --sql "select * from orders"
dbgraph validate-sql --file sql/orders.sql --dialect postgres --json
```

CLI and MCP require a local snapshot and graph index and apply the same syntax
and reference checks. Run `dbgraph snapshot` first; validation itself does not
connect to the database. A missing snapshot or index returns CLI exit code `3`.
`valid: true` requires no unresolved references or analysis diagnostics. Resolution
uses table aliases and query scopes: a column on another table cannot satisfy a
qualified reference. Unsupported SQL forms produce diagnostics rather than a
claim of complete validation. These checks do not prove that the database would
execute the SQL successfully, and they never execute it.

The CLI prints its report, then exits with code `2` when `valid` is false. Read
`diagnostics` and `unresolved` in JSON output before retrying with corrected SQL.

Compare latest snapshot with the previous snapshot:

```powershell
dbgraph diff
dbgraph diff --json
```

Diff requires two published snapshots. With zero or one snapshot it reports a
missing baseline instead of comparing a snapshot with itself.

Check impact before changing an object:

```powershell
dbgraph impact public.orders.status
dbgraph impact public.orders.status --depth 2 --json
```

## Analysis Reports

Run the structured analyzer:

```powershell
dbgraph analyze --scope all
dbgraph analyze --scope risk
dbgraph analyze --scope quality
dbgraph analyze --scope performance
```

Output formats:

```powershell
dbgraph analyze --scope all --format text
dbgraph analyze --scope all --format json
dbgraph analyze --scope all --json
dbgraph analyze --scope all --format markdown --output dbgraph-analysis.md
```

CI gates and known-risk suppressions:

```powershell
dbgraph analyze --fail-on high
dbgraph analyze --fail-on-new medium --baseline .dbgraph/analysis-baseline.json
dbgraph analyze --include-suppressed --suppressions .dbgraph/suppressions.json
```

The report includes:

- overview and risk score
- section summaries
- top findings
- severity counts
- evidence
- impact
- confidence
- suggested fixes
- related SQL/schema objects where available

Current rule groups:

- Security & Privacy: sensitive columns, SQL references to sensitive columns, broad `SELECT *`.
- Data Integrity & Schema Quality: missing primary keys, probable missing foreign keys.
- SQL Workload & Safety: `UPDATE` or `DELETE` statements without `WHERE`.
- Performance: filters or joins without supporting indexes.

Write-safety checks inspect the outer statement's AST condition. A `WHERE`
inside a nested query or a string literal does not restrict an outer `UPDATE`
or `DELETE` that has no condition.

## Offline Agent Benchmark

Compare raw project context with DbGraph structured context without calling an LLM:

```powershell
dbgraph benchmark-agent --scenario teashop --format markdown --output dbgraph-agent-benchmark.md
dbgraph benchmark-agent --scenario teashop --format json
```

The benchmark reports estimated tokens, retrieval steps, evidence recall, relevant object precision, and token reduction. See [agent-benchmark.md](agent-benchmark.md).

## MCP Server

Start the stdio MCP server:

```powershell
dbgraph serve --mcp
```

Available MCP tools:

- `dbgraph_status`
- `dbgraph_search`
- `dbgraph_table`
- `dbgraph_context`
- `dbgraph_relations`
- `dbgraph_impact`
- `dbgraph_analyze`
- `dbgraph_diff`
- `dbgraph_validate_sql`

MCP responses are JSON text content. Some detail tools include `responseBudget`
metadata when limiting collections; search/context use the coverage counts
described above.

Search/context `suggestedNextTools` contains at most two concrete
`{tool, arguments, reason}` entries. Pass `tool` as the next `tools/call.name` and
copy `arguments` unchanged; its `projectPath` preserves project scope even when
the server runs elsewhere. For example, after locating `public.orders` in a
project at `C:/work/shop`, a suggestion can be:

```json
{
  "tool": "dbgraph_table",
  "arguments": {"projectPath": "C:/work/shop", "table": "public.orders"},
  "reason": "inspect_snapshot_table"
}
```

The corresponding CLI path is `dbgraph table C:/work/shop public.orders --json`.
Use `dbgraph context C:/work/shop "orders status" --tokens 1600 --json` for the
task context. Other suggestions use `dbgraph_relations` with a real object ID,
or `dbgraph_validate_sql` with the snapshot's existing `normalizedSql` and an
explicit dialect; validation does not execute it. The CLI equivalents are
`dbgraph relations` and `dbgraph validate-sql`. Empty search candidates produce
no invented suggestions. Context can suggest a real seed omitted from its
final `objects` by the budget, making that evidence available for follow-up.
Every follow-up reads the latest snapshot at its own execution time; compare
report snapshot IDs if capture occurs between calls.

`mcp.maxResponseChars` caps the total Unicode character count of tool-result text.
It must be at least `64`. The JSON-RPC envelope
and transport escaping are outside this content budget. A result that still
exceeds the limit returns `isError: true` with JSON error kind
`response_budget_exceeded`; JSON is not truncated. Raise the limit in project
configuration or request less detail when the response cannot fit.

## PostgreSQL Teashop Smoke Test

Run the example database:

```powershell
docker compose -f examples/postgres-teashop/docker-compose.yml up -d
$env:DATABASE_URL="postgres://postgres:postgres@localhost:55432/teashop"
powershell -ExecutionPolicy Bypass -File scripts/integration/postgres-smoke.ps1
docker compose -f examples/postgres-teashop/docker-compose.yml down -v
```

The smoke test initializes a temporary project, captures a Postgres snapshot, runs search, validates SQL, and verifies the structured analysis report contains expected risk/performance findings and suggested fixes.

## Safety Notes

- `dbgraph validate-sql` never executes SQL.
- `dbgraph analyze` works from the local snapshot and graph index.
- `dbgraph snapshot` and `dbgraph sync` connect to capture metadata; `doctor --check-db` checks provider connectivity.
- Raw sample storage is off by default.
- Sensitive samples are masked when sampling is explicitly enabled.
- SQLite provider refuses to snapshot `.dbgraph/dbgraph.db`, so DbGraph's internal graph database is not confused with a business SQLite database.
