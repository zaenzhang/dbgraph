# DbGraph Offline Agent Benchmark

`dbgraph benchmark-agent` compares two ways an AI coding agent can gather database context:

| Mode | Meaning |
| --- | --- |
| Baseline | Raw project materials such as SQL, Markdown, JSON, and TOML files. |
| DbGraph | Structured snapshot, graph context, SQL lineage, and analysis findings. |

The benchmark is offline and deterministic. It does not call an LLM, send network requests, or measure provider billing tokens. Instead, it measures whether the context supplied to an agent contains the evidence needed to answer fixed database-review tasks.

## Run

After initializing and snapshotting a project:

```bash
dbgraph benchmark-agent --scenario teashop --format markdown --output dbgraph-agent-benchmark.md
```

JSON output is available for automation:

```bash
dbgraph benchmark-agent --scenario teashop --format json
```

## Metrics

| Metric | Meaning |
| --- | --- |
| Estimated tokens | Deterministic rough estimate from context size. |
| Context bytes | Raw bytes supplied to the simulated agent mode. |
| Retrieval steps | Approximate number of file/tool reads needed for the case. |
| Evidence recall | Fraction of expected objects present in the context. |
| Relevant object precision | Fraction of object-like mentions that match expected evidence. |
| Token reduction | Estimated token reduction from baseline to DbGraph mode. |

## Built-In Scenario

The first scenario is `teashop`. It covers:

- PII fields: `public.customers.email`, `public.payments.provider_token`
- SQL reads of sensitive columns
- `public.orders.status` quality and performance review
- schema quality findings
- join/filter index risk
- structured report completeness

The final numbers are computed from the current project files, latest snapshot, context builder, and analyzer output. They are not hardcoded sample scores.

## Latest Teashop Smoke Result

The current Postgres teashop smoke run produced:

| Metric | Baseline | DbGraph | Delta |
| --- | ---: | ---: | ---: |
| Estimated tokens | 67,326 | 6,324 | -90.6% |
| Retrieval steps | 12 | 12 | 0 |
| Evidence recall | baseline | +0.17 | +0.17 |
| Relevant object precision | baseline | +0.07 | +0.07 |

The benchmark verified evidence for:

- `public.customers.email`
- `public.payments.provider_token`
- `public.orders.status`

These values come from running `dbgraph benchmark-agent --scenario teashop --format markdown` during the Docker Postgres smoke test. They are useful documentation numbers for context size and evidence coverage, not live model billing or answer-quality measurements.

## Limitations

- This benchmark proves context quality and retrieval cost, not final model answer quality.
- Real LLM results can vary by model, prompt, temperature, and tool policy.
- Small schemas may show smaller savings than large schemas with many SQL artifacts.
- Baseline behavior is an approximation of raw file inspection, not a trace of one specific agent.
