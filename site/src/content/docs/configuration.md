---
title: Project Configuration
description: DbGraph project config, provider settings, security defaults, data access, MCP, and suppressions.
---

# DbGraph Project Configuration

DbGraph reads project configuration from:

```text
.dbgraph/dbgraph.config.json
```

Create it with:

```bash
dbgraph init -i --yes
```

DbGraph project state is local to the repository directory. The config controls the database provider, snapshot behavior, security defaults, and MCP response budget.

## Default Shape

```json
{
  "version": 1,
  "database": {
    "provider": "postgres",
    "connectionEnv": "DATABASE_URL",
    "connectionString": null
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
  },
  "dataAccess": {
    "defaultMode": "schemaOnly",
    "tables": []
  }
}
```

## `database`

| Field | Meaning |
| --- | --- |
| `provider` | Database provider. Supported values: `postgres`, `sqlite`, `mysql`, `sql-server`. PostgreSQL and SQLite are currently implemented. |
| `connectionEnv` | Environment variable containing the connection string. Preferred for secrets. |
| `connectionString` | Literal connection string fallback. Useful for local SQLite paths, but avoid plaintext secrets when possible. |

PostgreSQL:

```json
{
  "database": {
    "provider": "postgres",
    "connectionEnv": "DATABASE_URL",
    "connectionString": null
  }
}
```

SQLite:

```json
{
  "database": {
    "provider": "sqlite",
    "connectionEnv": null,
    "connectionString": "C:/path/to/app.sqlite"
  }
}
```

## `snapshot`

| Field | Meaning |
| --- | --- |
| `prettyJson` | Writes snapshot JSON in readable formatted form. |
| `profilingMode` | `schema`, `stats`, or `sample`. Defaults to `schema`. |
| `maxRowsPerTable` | Row limit used only when sample profiling is enabled. |
| `sampleRows` | Legacy opt-in flag. New configs should use `profilingMode: "sample"`. |

Profile modes:

- `schema`: schema-only metadata; safest default.
- `stats`: provider/catalog statistics such as row estimates.
- `sample`: allows bounded row sampling only when a matching `dataAccess` rule also allows it; values are masked by default.

CLI overrides:

```bash
dbgraph snapshot --profile schema
dbgraph snapshot --profile stats
dbgraph snapshot --profile sample --max-rows-per-table 20
```

## `security`

| Field | Meaning |
| --- | --- |
| `storeRawData` | Whether raw business row data may be stored. Defaults to `false`. |
| `storeRawSamples` | Whether raw sample values may be stored in sample mode. Defaults to `false`. |
| `maskPii` | Masks sensitive-looking values when sampling is explicitly enabled. Defaults to `true`. |
| `customSensitiveTerms` | Extra column-name terms to treat as sensitive, such as `employee_id` or `tax_id`. |

Safe default:

```json
{
  "security": {
    "storeRawData": false,
    "storeRawSamples": false,
    "maskPii": true,
    "customSensitiveTerms": ["tax_id", "employee_id"]
  }
}
```

Validation rules:

- `snapshot.sampleRows` requires `snapshot.profilingMode` to be `sample`.
- `security.storeRawSamples` requires `snapshot.profilingMode` to be `sample`.
- `snapshot.sampleRows` and `security.storeRawData` cannot both be true.

## `dataAccess`

`dataAccess` is the explicit allowlist for business-row access. Omitting it, or leaving `defaultMode` as `schemaOnly`, preserves the default behavior: DbGraph captures schema, constraints, indexes, SQL lineage, and safe statistics without reading row values.

Use this section when you want configuration-controlled database exposure. Each
table rule decides how deeply DbGraph may inspect matching tables:

- Keep sensitive or out-of-scope tables in `schemaOnly` so agents can still understand their shape and relationships without seeing row values.
- Use `stats` only for catalog/count-style provider metadata.
- Use `sample` only for tables and columns that are safe to inspect, with an optional read-only `where` filter and a strict `limit`.
- Downstream commands such as `context`, MCP tools, `analyze`, and `benchmark-agent` read from the snapshot and never expand access beyond the matched `dataAccess` rules.

Example:

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
        "columns": ["id", "status", "total_amount", "created_at"],
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

Modes:

| Mode | Behavior |
| --- | --- |
| `schemaOnly` | Keep schema, constraints, indexes, and SQL lineage; never read row values. |
| `stats` | Reserved for catalog/count-style stats; no row values. |
| `sample` | Read only the listed columns from matching tables with a deterministic `LIMIT` and optional read-only `where`. |

Validation rules:

- `sample` rules require `snapshot.profilingMode` to be `sample`.
- `sample.columns` must be non-empty.
- `limit` must be greater than zero and cannot exceed `snapshot.maxRowsPerTable`.
- `where` cannot contain semicolons or write-oriented SQL keywords.
- `storeRawValues` defaults to `false`; sensitive-looking values are still masked when `security.maskPii` is true.

Supported patterns are exact table names such as `public.orders` and simple `*` wildcards such as `public.audit_*`.

## `mcp`

| Field | Meaning |
| --- | --- |
| `enabled` | Whether MCP serving is enabled for the project. |
| `maxResponseChars` | Response size budget used by MCP tools before truncation/follow-up suggestions. |

Example:

```json
{
  "mcp": {
    "enabled": true,
    "maxResponseChars": 15000
  }
}
```

## Agent MCP Config

Project config is separate from agent config.

Run:

```bash
dbgraph install --target codex --yes
```

This writes an agent-side MCP entry similar to:

```json
{
  "mcpServers": {
    "dbgraph": {
      "command": "dbgraph",
      "args": ["serve", "--mcp"],
      "description": "DbGraph read-only database context for codex"
    }
  }
}
```

Supported targets:

```bash
dbgraph install --target codex --yes
dbgraph install --target cursor --yes
dbgraph install --target claude --yes
dbgraph install --target gemini --yes
dbgraph install --target opencode --yes
```

## Files Created

```text
your-project/
  .dbgraph/
    dbgraph.config.json
    snapshots/
    instructions/
      AGENTS.md.fragment
      CLAUDE.md.fragment
      dbgraph.mdc
    dbgraph.db
```

`dbgraph.db` is created after the first successful snapshot.

## Analysis Suppressions

Known accepted findings can be documented in:

```text
.dbgraph/suppressions.json
```

Example:

```json
{
  "version": 1,
  "suppressions": [
    {
      "ruleId": "quality.missing_primary_key",
      "object": "public.legacy_events",
      "reason": "Append-only import table",
      "owner": "data-platform",
      "expiresAt": "2026-12-31"
    }
  ]
}
```

Suppression matching is intentionally strict: `ruleId` and `object` must both match. Expired entries are reported as suppression warnings and do not hide findings.

Useful commands:

```bash
dbgraph analyze --include-suppressed
dbgraph analyze --suppressions .dbgraph/suppressions.json
dbgraph analyze --fail-on high
dbgraph analyze --fail-on-new medium --baseline .dbgraph/analysis-baseline.json
```

By default, active `findings` excludes suppressed findings. JSON output also includes `suppressedFindings`, `suppressionCounts`, and `gate`.
