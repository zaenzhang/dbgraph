# DbGraph

DbGraph is a local-first database context engine for AI coding agents. It builds
a local graph of database schema objects, SQL artifacts, and relationships so an
agent can search, validate, and reason about database changes without storing
business row data by default.

## What Works Now

### Project Initialization

- `dbgraph init [PATH]` creates local `.dbgraph/` project state.
- `dbgraph init -i` supports an interactive setup flow.
- `dbgraph init -i --yes` uses default interactive options without prompts.
- Optional agent instruction fragments can be generated under `.dbgraph/instructions/`.
- `dbgraph status [PATH] [--json]` reports initialization, config, snapshot, and graph index state.

### PostgreSQL Snapshot

- `dbgraph snapshot [PATH] [--json]` captures PostgreSQL schema metadata.
- The provider currently supports PostgreSQL only.
- The snapshot includes schemas, tables, columns, constraints, indexes, views,
  materialized views, routines, triggers, enums, sequences, and statistics where available.
- Sensitive connection strings are not printed in provider errors.
- Snapshot JSON is written under `.dbgraph/snapshots/`.
- A local SQLite graph index is rebuilt after snapshot capture.

### Storage and Graph Index

- SQLite storage tracks snapshots, objects, edges, table profiles, column profiles,
  SQL artifacts, and project metadata.
- Object search uses FTS5 when available, with a fallback search path.
- Graph edges include containment, column ownership, constraints, indexes,
  explicit references, inferred references, SQL reads/writes/joins/filters, and dependencies.
- The graph builder supports inferred relations from naming patterns such as
  `orders.user_id -> users.id`.

### SQL Analysis

- `dbgraph-sql` wraps `sqlparser-rs` with PostgreSQL, MySQL, and generic dialect selection.
- SQL parsing preserves raw SQL, normalized SQL, fingerprints, parsed statement summaries,
  and diagnostics.
- SQL file scanning finds `.sql` files under `migrations/`, `sql/`, and `db/` by default.
- Scanner results are reproducible and ignore noisy directories such as `node_modules`,
  `target`, `bin`, and `obj`.
- SQL lineage extraction currently supports:
  - `reads_from`
  - `writes_to`
  - `joins_on`
  - `filters_by`
  - `groups_by`
  - `orders_by`
  - `depends_on` for CTEs
- SQL artifacts are written into SQLite and also become query objects in the graph.
- SQL artifact fingerprints prevent duplicate storage for the same snapshot.

### SQL Validation

- `dbgraph validate-sql [PATH] --sql "<SQL>" [--dialect postgres|mysql|generic] [--json]`
  validates an inline SQL string.
- `dbgraph validate-sql [PATH] --file path/to/query.sql [--json]` validates a SQL file.
- Validation parses SQL and checks referenced tables/columns against the local graph index.
- It reports unresolved objects and suggests related local graph objects.
- It does not execute SQL, apply migrations, or connect to the business database.

## Current CLI

```bash
dbgraph --version
dbgraph --help
dbgraph init [PATH] [--force] [-i|--interactive] [--yes]
dbgraph status [PATH] [--json]
dbgraph snapshot [PATH] [--json]
dbgraph validate-sql [PATH] (--sql SQL | --file FILE) [--dialect postgres|mysql|generic] [--json]
```

## What We Do Next

The next planned phase is Phase 05: Context and Impact.

The first task is `DBG-0501`, which adds:

- `dbgraph search`
- keyword search over tables, columns, constraints, views, and SQL artifacts
- object kind filtering
- stable result ordering
- JSON output
- a clear message when no graph index exists yet

After search, the planned Phase 05 work continues with:

- `dbgraph table` for detailed table structure and relationships
- `dbgraph relations` for incoming, outgoing, explicit, and inferred relations
- context candidate retrieval and ranking for AI tasks
- impact analysis over schema objects and SQL artifacts

## Development

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --workspace
```

## Safety Defaults

- DbGraph is local-first.
- It stores schema metadata and SQL artifacts, not business row data by default.
- SQL validation is parse-only and graph-only.
- Snapshot currently connects only when explicitly requested through `dbgraph snapshot`.

