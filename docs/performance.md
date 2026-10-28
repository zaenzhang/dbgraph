# Performance And Scale

Use the built-in benchmark command to generate deterministic synthetic schemas:

```powershell
dbgraph benchmark --tables 1000 --columns-per-table 4 --json
dbgraph benchmark --tables 10000 --columns-per-table 2 --json
```

Storage rebuilds use a single SQLite transaction and batched prepared statements. The test suite includes a 10k-object storage rebuild and a 10k-table synthetic schema generation check.

## Context Evidence Slice — 2026-10-28

Measured on Windows x64 with Rust 1.95.0, an unoptimized local CLI/MCP build,
and an isolated SQLite fixture containing 28 captured objects (5 tables and 2
SQL files). No model service or business database was used. The baseline was
the official v0.1.4 Windows x64 release binary, querying the same fixture.
Its optimized build differs from the local build, so elapsed times below are
observations of current calls, not a baseline latency comparison.

Response sizes count complete pretty JSON tool text in UTF-8, excluding the
JSON-RPC envelope and transport escaping. Tokens are `ceil(bytes / 4)`, not model
tokenizer or billing values.

| Same request | v0.1.4 bytes / tokens | Current bytes / tokens | Returned evidence |
| --- | --- | --- | --- |
| search `kind:table 退款` | 241 / 61 | 1126 / 282 | Both return `main.refunds`; current adds provenance, lexical coverage and a callable scoped table expansion. |
| search `不存在的概念` | 52 / 13 | 596 / 149 | Both return no objects; current explicitly reports 28 examined candidates and no lexical evidence, with no invented suggestion. |
| context `退款 payment`, budget 800 | 3144 / 786 | 3198 / 800 | Old: 12 objects / 11 paths; current: 6 objects / 4 paths plus provenance and diagnostics. Current reports 8 budget-omitted candidates, 1 omitted edge among returned objects, and 1 omitted suggestion. |

The normal one-table hit adds **885 bytes / 221 estimated tokens**. The current
800-token context retains both relevant tables and the refund foreign-key path,
but exposes fewer column/constraint details. Follow-up expansion can recover
details; the test does not establish that the tradeoff reduces total calls or
improves full evidence recall. Generic context summaries that merely repeated
kind/name are omitted; captured comments/descriptions remain.

The tool registry remains at 9 tools. Compact `tools/list.result` measured 6343
bytes / 1586 estimated tokens for v0.1.4 and 6496 / 1624 for the current build:
153 bytes / 38 estimated tokens added to describe the actual contract.

### Observed Dependent Journey

Question: locate the refund table and validate its captured SQL. The server cwd
was the temporary parent, outside the project. Every call carried the fixture's
absolute `projectPath`; generated arguments were copied unchanged. After table
retrieval, its returned short name supplied the next query's vocabulary.

| Actual call, in order | Specific input (project path abbreviated) | Bytes / tokens | Elapsed ms |
| --- | --- | --- | --- |
| `dbgraph_search` | `query: "kind:table 退款"` | 1126 / 282 | 1.812 |
| `dbgraph_table` | returned `table: "main.refunds"` | 2277 / 570 | 1.372 |
| `dbgraph_context` | `query: "kind:query refunds", tokenBudget: 800` | 2668 / 667 | 2.574 |
| `dbgraph_validate_sql` | returned `sql: "SELECT r.id, r.amount FROM refunds AS r", dialect: "generic"` | 531 / 133 | 1.359 |

This recorded journey used 4 actual calls, 6602 response bytes and 1652 summed
per-call estimated tokens. SQL validation returned `valid: true`, `executed:
false`; context retained the existing `sql/refunds.sql` source. This is a
completed path, not evidence of fewer calls than a baseline agent. The no-answer
search returned 596 bytes / 149 tokens in 1.745 ms and offered no guessed object.
The scoring checks ran after retrieval and never filtered its output.

The final acceptance script recorded 66 CLI/MCP operations including setup,
boundary checks, suggestion calls and six baseline/current size samples. It
records real operations rather than assigning a fixed retrieval-call count.
Successful MCP calls in that run:

| Tool | Samples | Min / median / max ms |
| --- | --- | --- |
| search | 13 | 1.061 / 1.433 / 2.430 |
| context | 7 | 1.103 / 2.574 / 8.115 |
| table | 7 | 1.310 / 1.372 / 1.623 |
| relations | 3 | 1.141 / 1.214 / 1.453 |
| validate SQL | 3 | 1.359 / 1.609 / 2.233 |

These small local samples do not establish percentiles, cold-disk behavior,
production scale, PostgreSQL performance, task accuracy with a model, or cost
savings. No strict performance threshold, new evaluation platform, cache,
vector store, or FTS switch was introduced. Context now creates object and
adjacency maps per request, instead of repeatedly finding objects or scanning
all edges for each selected object.

Reproduce using [the offline acceptance script](integration-tests.md#offline-context-evidence-acceptance).
The original `benchmark-agent` still uses expected objects to select findings
and fixed retrieval-step estimates; it was not used as end-to-end proof here.
Raw temporary reports and downloaded tools are removed after the requested
cleanup; this summary and the reproducible script remain. Findings and execution
boundaries belong to [the slice plan](context-evidence-plan.md).
