# Docker Integration Tests

The repository includes a PostgreSQL teashop example that can be run with Docker.

```powershell
docker compose -f examples/postgres-teashop/docker-compose.yml up -d
$env:DATABASE_URL="postgres://postgres:postgres@localhost:55432/teashop"
powershell -ExecutionPolicy Bypass -File scripts/integration/postgres-smoke.ps1
docker compose -f examples/postgres-teashop/docker-compose.yml down -v
```

The smoke script initializes a temporary DbGraph project, captures a PostgreSQL snapshot, runs graph queries, and validates a SQL artifact.

## Offline Context Evidence Acceptance

Build the CLI, then run the disposable SQLite CLI/MCP smoke (Python standard library only):

```powershell
cargo build -p dbgraph-cli --offline
python scripts/integration/context-evidence-smoke.py --bin target/debug/dbgraph.exe --output reports/context-evidence-smoke.json
```

Use `target/debug/dbgraph` on Linux/macOS. Optional `--baseline-bin PATH` runs a
published older binary against the same fixture for payload-size comparison;
different build profiles do not establish a latency improvement.

The script records actual calls, response bytes/rough tokens, elapsed time and a
dependent search → suggested table expansion → captured SQL context → suggested
SQL validation journey. Expected evidence is asserted after retrieval and never
selects candidates. It exercises honest empty/missing/filter/Unicode/budget
responses and same-parameter CLI/MCP diagnostics. An empty SQLite database still
captures the `main` schema; a separate synthetic zero-object snapshot and matching
index exercise `empty_snapshot`. No business SQL is executed. Temporary projects
and the MCP process are closed; remove the output report when no longer needed.

The 2026-10-28 acceptance scope, discoveries and cleanup are recorded in
[context-evidence-plan.md](context-evidence-plan.md).
