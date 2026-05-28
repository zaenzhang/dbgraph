# CLI and Provider Optimization Design

## Goal

Improve Phase 03 maintainability and first-run usability without changing the snapshot data model or starting Phase 04.

## Scope

1. Split `crates/dbgraph-provider/src/lib.rs` into focused modules:
   - `types.rs` for provider traits and raw snapshot structs.
   - `registry.rs` for provider lookup.
   - `postgres/connection.rs` for connection, timeout, read-only setup, and redaction.
   - `postgres/extract.rs` for PostgreSQL catalog queries.
   - `postgres/canonicalize.rs` for raw catalog to canonical snapshot mapping.
   - `postgres/raw.rs` for PostgreSQL raw row types.
2. Make `dbgraph init -i` run snapshot immediately when the user answers yes.
3. Polish CLI output and error messages for snapshot-first usage.

## Non-Goals

- Do not add Phase 04 SQL analysis.
- Do not add new query/search commands.
- Do not change the canonical `DbSnapshot` model unless a compile-safe refactor requires imports to move.
- Do not introduce hardcoded behavior just to satisfy tests.

## Behavior

Provider refactor must be behavior-preserving. Existing provider and CLI tests remain the primary safety net.

For `init -i`, choosing `Run snapshot now? yes` initializes project files first and then runs the same snapshot flow as `dbgraph snapshot`. If snapshot fails, the error is returned to the user and the initialized project files remain on disk.

Snapshot output should remain JSON-compatible for `--json` and become clearer for human output.

## Verification

Run:

```powershell
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --workspace
```

Provider live PostgreSQL verification remains optional and requires Docker or `DBG_TEST_DATABASE_URL`.
