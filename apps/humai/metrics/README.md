This folder stores local metrics/tracing artifacts for offline analysis.

Current tracing output:

- `timings.jsonl` (default)

Use the built-in reporter binary to summarize model performance and run behavior:

```sh
# default input: metrics/timings.jsonl
cargo run --bin timings_report

# custom file
cargo run --bin timings_report -- metrics/timings.jsonl
```

This folder also contains the *.sqlite file. 