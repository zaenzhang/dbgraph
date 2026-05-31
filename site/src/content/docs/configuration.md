---
title: Project Configuration
description: DbGraph project config, provider settings, security defaults, MCP, and suppressions.
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
- `sample`: explicit opt-in sampling; values are masked by default.

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
