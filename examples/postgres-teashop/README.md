# PostgreSQL Teashop Example

This example provides a small commerce schema for DbGraph demos.

```powershell
docker compose -f examples/postgres-teashop/docker-compose.yml up -d
$env:DATABASE_URL="postgres://postgres:postgres@localhost:55432/teashop"
dbgraph init -i --yes
dbgraph snapshot --profile stats
dbgraph search order --kind table
dbgraph context "customer order payment"
dbgraph analyze --scope all --format markdown --output teashop-analysis.md
```

The `sql/orders.sql` file is discovered during snapshot and appears as SQL artifact/query graph context.
The analysis report should include sensitive column findings for customer email and payment provider tokens, plus a performance finding for `public.orders.status`.

To test allowlisted data profiling, set `.dbgraph/dbgraph.config.json` to sample mode and add:

```json
{
  "dataAccess": {
    "defaultMode": "schemaOnly",
    "tables": [
      {
        "pattern": "public.orders",
        "mode": "sample",
        "columns": ["status", "created_at"],
        "where": "created_at >= now() - interval '30 days'",
        "limit": 10,
        "storeRawValues": true
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
dbgraph analyze --scope all --format markdown
```

The report should include a `Data Profiling & Business Rules` finding for `public.orders.status`, while `public.payments` stays schema-only.

Shut it down with:

```powershell
docker compose -f examples/postgres-teashop/docker-compose.yml down -v
```
