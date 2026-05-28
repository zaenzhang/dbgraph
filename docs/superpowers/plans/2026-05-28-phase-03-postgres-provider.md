# Phase 03 PostgreSQL Provider Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build Phase 03 so DbGraph can connect to PostgreSQL, collect read-only catalog metadata, produce canonical snapshots, write JSON snapshots, and rebuild the local SQLite graph index.

**Architecture:** `dbgraph-provider` owns provider traits, raw snapshot structs, registry, and a concrete synchronous PostgreSQL provider. The CLI remains provider-agnostic at the command boundary and only uses registry/provider traits plus Phase 02 snapshot/index APIs.

**Tech Stack:** Rust workspace, `postgres` sync client with TLS disabled by default, `url` for safe connection string database-name extraction, `serde` for raw/canonical data, `rusqlite` Phase 02 index, Cargo unit tests plus optional live PostgreSQL integration tests gated by `DBG_TEST_DATABASE_URL`.

---

## File Structure

- Modify `crates/dbgraph-provider/Cargo.toml`: add `dbgraph-core`, `postgres`, `url`, `serde`, `serde_json`, and test dependencies if needed.
- Replace `crates/dbgraph-provider/src/lib.rs`: provider trait, registry, connection config, raw PostgreSQL row models, extractor, canonical mapper, and tests.
- Modify `crates/dbgraph-cli/Cargo.toml`: add `dbgraph-provider`, `dbgraph-storage`, `dbgraph-graph`, and keep existing serde deps.
- Modify `crates/dbgraph-cli/src/main.rs`: add `snapshot` command parsing, config loading, provider invocation, snapshot JSON write, SQLite index rebuild, and summary output.
- Optionally modify `crates/dbgraph-core/src/model.rs`: only if Phase 03 metadata needs fields already implied by Phase 02 but not expressible.

## Task 1: Provider Abstraction and Registry

**Files:**
- Modify: `crates/dbgraph-provider/Cargo.toml`
- Modify: `crates/dbgraph-provider/src/lib.rs`

- [ ] **Step 1: Write failing tests**

Add tests in `crates/dbgraph-provider/src/lib.rs`:

```rust
#[test]
fn registry_resolves_postgres_without_exposing_concrete_type() {
    let registry = ProviderRegistry::default();
    let provider = registry.get("postgres").expect("postgres provider should exist");
    assert_eq!(provider.id(), "postgres");
    assert_eq!(provider.capabilities().schema_metadata, CapabilityStatus::Supported);
}

#[test]
fn raw_snapshot_capabilities_serialize_to_metadata() {
    let snapshot = RawSchemaSnapshot {
        provider: "postgres".to_owned(),
        database_name: "app".to_owned(),
        capabilities: ProviderCapabilities::default(),
        ..RawSchemaSnapshot::default()
    };
    let json = serde_json::to_string(&snapshot).expect("raw snapshot should serialize");
    assert!(json.contains("capabilities"));
}
```

- [ ] **Step 2: Run failing tests**

Run: `cargo test -p dbgraph-provider registry_resolves_postgres raw_snapshot_capabilities`

Expected: FAIL because `ProviderRegistry` and `RawSchemaSnapshot` do not exist.

- [ ] **Step 3: Implement minimal provider abstraction**

Define:
- `DatabaseProvider` trait with `id`, `capabilities`, `connect`, `snapshot`.
- `ProviderRegistry` returning `Box<dyn DatabaseProvider>`.
- `RawSchemaSnapshot`, `RawStatisticsSnapshot`, `RawTable`, `RawColumn`, `RawConstraint`, `RawIndex`, `RawRoutine`, `RawTrigger`, `RawEnum`, `RawSequence`.
- `ProviderConnectionConfig` with URL redaction helpers.

- [ ] **Step 4: Run tests**

Run: `cargo test -p dbgraph-provider registry_resolves_postgres raw_snapshot_capabilities`

Expected: PASS.

## Task 2: PostgreSQL Connection and Read-Only Safety

**Files:**
- Modify: `crates/dbgraph-provider/src/lib.rs`

- [ ] **Step 1: Write failing tests**

Add tests:

```rust
#[test]
fn connection_error_does_not_include_password() {
    let config = ProviderConnectionConfig {
        url: "postgres://user:secret@127.0.0.1:1/missing".to_owned(),
        connect_timeout_ms: 50,
        statement_timeout_ms: 1000,
    };
    let err = PostgresProvider::default().connect(&config).expect_err("connect should fail");
    let message = err.to_string();
    assert!(message.contains("failed to connect to PostgreSQL"));
    assert!(!message.contains("secret"));
}

#[test]
fn database_name_is_read_from_connection_url_without_password_leak() {
    let config = ProviderConnectionConfig::from_url("postgres://user:secret@localhost/app_db");
    assert_eq!(config.database_name_hint().as_deref(), Some("app_db"));
    assert!(!config.redacted_url().contains("secret"));
}
```

- [ ] **Step 2: Run failing tests**

Run: `cargo test -p dbgraph-provider connection_error_does_not_include_password database_name_is_read`

Expected: FAIL because connection helpers are incomplete.

- [ ] **Step 3: Implement connection**

Use `postgres::Config` parsed from URL, set connect timeout, connect with `NoTls`, issue only setup/catalog statements:
- `SET statement_timeout = <ms>`
- `SHOW transaction_read_only`
- `SELECT current_user, version(), current_database()`
- Optional readonly probe inside `BEGIN READ ONLY`

- [ ] **Step 4: Run tests**

Run: `cargo test -p dbgraph-provider connection_error_does_not_include_password database_name_is_read`

Expected: PASS.

## Task 3: PostgreSQL Schema/Table/Column/Comment Extraction

**Files:**
- Modify: `crates/dbgraph-provider/src/lib.rs`

- [ ] **Step 1: Write mapper tests before live queries**

Add a test that constructs raw schema/table/column rows and converts them to `DbSnapshot`, asserting:
- `public` and custom schema objects exist.
- system schemas are excluded.
- `data_type` and `data_type_family` are preserved.
- table and column comments appear in metadata.

- [ ] **Step 2: Run failing test**

Run: `cargo test -p dbgraph-provider maps_schema_tables_columns_and_comments`

Expected: FAIL because mapper is incomplete.

- [ ] **Step 3: Implement catalog query and mapper**

Query only catalog/information schema:
- schemas from `pg_namespace`
- tables from `pg_class` and `pg_namespace`
- comments through `obj_description` / `col_description`
- columns from `pg_attribute`, `pg_type`, `pg_attrdef`
Filter `pg_catalog`, `information_schema`, `pg_toast`, and temp schemas.

- [ ] **Step 4: Run tests**

Run: `cargo test -p dbgraph-provider maps_schema_tables_columns_and_comments`

Expected: PASS.

## Task 4: Constraints and Foreign-Key Edges

**Files:**
- Modify: `crates/dbgraph-provider/src/lib.rs`

- [ ] **Step 1: Write failing tests**

Add mapper test with composite primary key, composite foreign key, unique, and check constraints. Assert:
- constraint objects have correct kind.
- constraint metadata contains schema/table/columns.
- FK `references` edge direction is from source FK/column-side object to referenced table/columns.
- delete/update actions are stored in metadata.

- [ ] **Step 2: Run failing test**

Run: `cargo test -p dbgraph-provider maps_constraints_and_foreign_key_edges`

Expected: FAIL.

- [ ] **Step 3: Implement constraint extraction**

Query `pg_constraint`, `unnest(conkey) WITH ORDINALITY`, `unnest(confkey) WITH ORDINALITY`, and map:
- `p` -> primary key
- `f` -> foreign key plus `references` edge
- `u` -> unique constraint
- `c` -> check constraint

- [ ] **Step 4: Run tests**

Run: `cargo test -p dbgraph-provider maps_constraints_and_foreign_key_edges`

Expected: PASS.

## Task 5: Advanced Objects

**Files:**
- Modify: `crates/dbgraph-provider/src/lib.rs`

- [ ] **Step 1: Write failing tests**

Add mapper tests for indexes, views, functions, procedures, triggers, enums, and sequences. Assert:
- index columns/expression/predicate stored.
- view/function/trigger objects exist.
- trigger produces `triggered_by` edge.

- [ ] **Step 2: Run failing test**

Run: `cargo test -p dbgraph-provider maps_indexes_views_routines_triggers_enums_sequences`

Expected: FAIL.

- [ ] **Step 3: Implement advanced catalog extraction**

Use catalog-only queries:
- indexes from `pg_index`, `pg_class`, `pg_get_indexdef`
- views from `pg_views`
- routines from `pg_proc`
- triggers from `pg_trigger`
- enums from `pg_enum`
- sequences from `pg_class relkind = 'S'`

- [ ] **Step 4: Run tests**

Run: `cargo test -p dbgraph-provider maps_indexes_views_routines_triggers_enums_sequences`

Expected: PASS.

## Task 6: Statistics Without Full Table Scan

**Files:**
- Modify: `crates/dbgraph-provider/src/lib.rs`

- [ ] **Step 1: Write failing tests**

Add test that maps `reltuples`, `pg_total_relation_size`, and `pg_stats` rows into table/column profiles. Assert:
- table profiles include `row_count_kind = estimate`.
- profile metadata includes source.
- column profiles include null fraction, distinct estimate, avg width.
- most common values are not stored raw.

- [ ] **Step 2: Run failing test**

Run: `cargo test -p dbgraph-provider maps_statistics_without_sensitive_values`

Expected: FAIL.

- [ ] **Step 3: Implement stats extraction**

Use only:
- `pg_class.reltuples`
- `pg_total_relation_size(oid)`
- `pg_stats.null_frac`, `n_distinct`, `avg_width`, histogram metadata summaries
Do not run `count(*)` or select business rows.

- [ ] **Step 4: Run tests**

Run: `cargo test -p dbgraph-provider maps_statistics_without_sensitive_values`

Expected: PASS.

## Task 7: CLI Snapshot Command

**Files:**
- Modify: `crates/dbgraph-cli/Cargo.toml`
- Modify: `crates/dbgraph-cli/src/main.rs`

- [ ] **Step 1: Write failing CLI tests**

Add parse and safe failure tests:

```rust
#[test]
fn parses_snapshot_command() {
    let parsed = parse(&["snapshot", "--json"]).expect("args should parse");
    assert_eq!(parsed.command, Command::Snapshot { path: None, json: true });
}

#[test]
fn snapshot_requires_initialized_project() {
    let temp = TempProject::new();
    let err = run(["snapshot".to_owned(), temp.root.display().to_string()])
        .expect_err("snapshot should require config");
    assert!(err.to_string().contains("Run `dbgraph init` first"));
}
```

- [ ] **Step 2: Run failing tests**

Run: `cargo test -p dbgraph-cli parses_snapshot_command snapshot_requires_initialized_project`

Expected: FAIL.

- [ ] **Step 3: Implement CLI command**

Add `snapshot [PATH] [--json]`:
- load `.dbgraph/dbgraph.config.json`
- reject non-postgres providers for Phase 03
- resolve connection URL from env or explicit config
- call provider snapshot
- canonicalize snapshot
- write JSON via `SnapshotStore`
- rebuild SQLite via `GraphRepository` and `dbgraph_graph::rebuild_index`
- print summary with table/column/edge counts and output paths

- [ ] **Step 4: Run tests**

Run: `cargo test -p dbgraph-cli parses_snapshot_command snapshot_requires_initialized_project`

Expected: PASS.

## Task 8: Optional Live Integration Test and Final Verification

**Files:**
- Modify: `crates/dbgraph-provider/src/lib.rs`
- Modify: `crates/dbgraph-cli/src/main.rs`

- [ ] **Step 1: Add ignored/gated live test**

Add a test that only runs when `DBG_TEST_DATABASE_URL` is set. It should create a tiny fixture schema in a temporary schema, run provider snapshot, and assert table/column/FK/view/index/profile counts. Keep it ignored or env-gated so CI without Postgres passes.

- [ ] **Step 2: Run complete verification**

Run:

```powershell
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --workspace
```

Expected: all exit 0.

- [ ] **Step 3: Completion audit**

Check DBG-0301 through DBG-0307 one by one against code and tests. If `DBG_TEST_DATABASE_URL` is absent, report that live PostgreSQL verification was not run and keep the implementation ready for env-backed testing.
