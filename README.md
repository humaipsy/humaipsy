# Humai

[![Rust](https://img.shields.io/badge/Rust-000000?style=flat&logo=rust&logoColor=white)](https://www.rust-lang.org) [![License: AGPL v3](https://img.shields.io/badge/License-AGPLv3-blue.svg)](LICENSE) [![CI](https://github.com/vecnode/humai/actions/workflows/ci.yml/badge.svg)](https://github.com/vecnode/humai/actions/workflows/ci.yml) [![Website](https://img.shields.io/website?url=https%3A%2F%2Fvecnode.github.io%2Fhumai%2F&label=website)](https://vecnode.github.io/humai/)

🤖 Humai, application for running scientific Human-AI-agent Interaction experimentation for experimental psychology, cognitive sciences and HCI research. 

<img src="./assets/panel_1.png" alt="Humai panel 1" width="80%">
<img src="./assets/panel_2.png" alt="Humai panel 2" width="80%">
<img src="./assets/panel_3.png" alt="Humai panel 3" width="80%">

Features:

- Cross-platform desktop application (Windows, Linux, macOS) for Human-AI Interaction research
- Runs a multi-condition (e.g., Cooperative / Argumentative), counterbalanced chat experiment against a local or remote [Ollama](https://ollama.com) model, with a researcher-configurable number of trials per condition
- Local and offline-first: no telemetry, no required network dependency beyond the Ollama host you point it at. An optional local read-only web dashboard (loopback-only by default) can be viewed in a browser while the app is running
- Every trial is recorded to CSV, a mirrored JSONL audit-trail ledger, and a local SQLite chat log

For how the app is put together (process model, module layout, storage formats, known limitations, and what's planned next) see [ARCHITECTURE.md](ARCHITECTURE.md). For contributor rules (build/run/test commands, cross-platform gotchas, release process) see [AGENTS.md](AGENTS.md). For what's actually protected (and what isn't) if you're handling real participant data, see [SECURITY.md](SECURITY.md).

The engine (agent prompting, Ollama client, experiment recording, chat store, vault) lives in the `humai-core` library crate with no UI dependency; `apps/humai` is the `eframe`/`egui` desktop binary built on top of it. We also add a live viewer for a secondary screen.

<img src="./assets/live_view.png" alt="Humai live viewer" width="80%">

## Dependencies

- Git (>= 2.30)
- Rust toolchain (stable with Edition 2024 support; rustc >= 1.85)
- Cargo (bundled with Rust toolchain)
- **Linux**: build-essential (or equivalent), pkg-config, and the X11/Wayland/audio
  dev headers eframe's windowing backend links against: `libxkbcommon-dev`,
  `libwayland-dev`, `wayland-protocols`, `libx11-dev`, `libxrandr-dev`, `libxi-dev`,
  `libxcursor-dev`, `libasound2-dev`, `libegl1-mesa-dev`, `libgl1-mesa-dev` (Debian/Ubuntu
  package names — see `.github/workflows/ci.yml` for the exact `apt-get install` list)
- **Windows**: Microsoft C++ Build Tools (the MSVC linker `link.exe`), via the
  [Visual Studio Build Tools installer](https://visualstudio.microsoft.com/visual-cpp-build-tools/) —
  required by the default `x86_64-pc-windows-msvc` Rust target
- **macOS**: Xcode Command Line Tools (`xcode-select --install`) for the system linker/toolchain

## Components

- [humai](./apps/humai/)
- [humai-web](./crates/humai-web/)
- [humai-core](./crates/humai-core/)

## Development

<details>
<summary>Linux</summary>
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


<details>
<summary>Windows</summary>
<br>

```sh
# Rust Application Development
run_app.bat

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


<details>
<summary>MacOS</summary>
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

Copyright (C) 2026 Luís Arandas and Olga Sutskova

Licensed under the GNU Affero General Public License v3.0 (AGPL-3.0) — see [LICENSE](LICENSE) for the full text. 