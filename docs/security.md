# DbGraph Security Notes

DbGraph is designed to be schema-first and local by default.

## Defaults

- Raw business rows are not stored.
- Raw samples are not stored.
- PII masking is enabled.
- `snapshot.profilingMode` defaults to `schema`.
- Sampling requires both `profilingMode: "sample"` or `dbgraph snapshot --profile sample` and a matching `dataAccess` allowlist rule.

## PII Detection

PII scoring uses column name, type, comments, metadata comments, and custom terms:

```json
{
  "security": {
    "storeRawData": false,
    "storeRawSamples": false,
    "maskPii": true,
    "customSensitiveTerms": ["tax_id", "national_id"]
  }
}
```

Sensitive columns receive a `piiScore` in column profiles. Sample summaries mask sensitive values even when raw sample storage is explicitly enabled.

## Sampling Policy

Safe sampling is allowlist-only. DbGraph reads business row values only during `dbgraph snapshot --profile sample` or equivalent sample-mode config, and only for columns listed in a matching `dataAccess.tables[]` rule.

Sampling is bounded by `snapshot.maxRowsPerTable` and by any lower per-rule `limit`. PostgreSQL and SQLite use deterministic `LIMIT` sampling with an optional configured read-only `where` clause. Stored sample summaries contain counts, masked examples, inferred shape, basic numeric range, observed format shapes, and source metadata.

Raw examples are not stored unless a table rule sets `storeRawValues: true`. Sensitive-looking values are still masked when `security.maskPii` is true.

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

`schemaOnly` tables keep schema and SQL lineage analysis but never read row values.

## Local Files To Protect

- `.dbgraph/dbgraph.config.json` may contain connection details if `connectionString` is used.
- `.dbgraph/snapshots/*.json` contains schema metadata and profile summaries.
- `.dbgraph/dbgraph.db` contains the local graph index.

Prefer `connectionEnv` over plaintext `connectionString` when possible.
