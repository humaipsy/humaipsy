# Humai v.0.1

(Under heavy development)

Features:

- Cross-platform desktop application (Windows, Linux, macOS) for Human-AI Interaction research
- Runs a two-condition (Cooperative / Argumentative), counterbalanced chat experiment against a
  local or remote [Ollama](https://ollama.com) model, with a researcher-configurable number of
  trials per condition
- Local and offline-first: no server component, no telemetry, no required network dependency
  beyond the Ollama host you point it at
- Every trial is recorded to CSV, a mirrored JSONL audit-trail ledger, and a local SQLite chat log

For how the app is put together (process model, module layout, storage formats, known
limitations, and what's planned next) see [ARCHITECTURE.md](ARCHITECTURE.md). For contributor
rules (build/run/test commands, cross-platform gotchas, release process) see
[AGENTS.md](AGENTS.md).

The engine (agent prompting, the Ollama client, experiment recording, the chat store, the vault)
lives in the `humai-core` library crate with no UI dependency; `apps/humai` is the `eframe`/`egui`
desktop binary built on top of it.

## Dependencies

- Git (>= 2.30)
- Rust toolchain (stable with Edition 2024 support; rustc >= 1.85)
- Cargo (bundled with Rust toolchain)


## Development

<details>
<summary>Commands</summary>
<br>

```sh
# Rust Application Development
./run_app.sh

# Manually
cd ./apps/humai

# Fast compile check (all bins and tests)
cargo check --all-targets

# Run app (UI only)
cargo run

# Run the helper binary to create/update vault/.master_hash (run from apps/humai/)
cargo run --bin gen_master_hash

# Build release binary
cargo build --release

# Run tests (add --workspace from the repo root to also cover the humai-core crate)
cargo test

# Lint — CI-equivalent gate, must pass clean
cargo clippy --all-targets -- -D warnings

```

</details>



## License

Proprietary
