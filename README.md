# HumxAI (Humai) v0.1

Under heavy development. 

Repository for HumxAI (Humai) v0.1.

- Cross-platform Application for Human-AI Interaction
- Design Experimental Tests
- Integrated with open-weight models 
- Local and offline first-field architecture


## Installation Windows/Linux

Installation distribution.

<details>
<summary>Distribute</summary>
<br>
</details>

## Source

Hello.

<details>
<summary>Architecture</summary>
<br>
</details>

## Dependencies

- Git (>= 2.30)
- Docker Engine (>= 24.0)
- Docker Compose plugin (>= 2.20, Compose V2)
- Rust toolchain (stable with Edition 2024 support; rustc >= 1.85)
- Cargo (bundled with Rust toolchain)
- Python (>= 3.12)
- uv (latest stable)



## Reproduce

Development 

<details>
<summary>Commands</summary>
<br>

```sh
# Rust Application Development
cd ./arp/apps/arpsci

# Fast compile check (all bins and tests)
cargo check --all-targets

# Run app (UI only)
cargo run

# Run app with embedded Rocket API enabled
ARPSCI_WEB_ENABLED=true cargo run

# Run the helper binary to create/update runs/.master_hash (run from repo root)
cargo run --bin gen_master_hash

# Build release binary
cargo build --release

# Run tests
cargo test

# Format and lint (if needed before commit)
cargo fmt
cargo clippy --all-targets -- -D warnings

# Run Platform
./run_all.sh

# Terminal 1:
cd ./arp/platform
uv sync --dev
uv run uvicorn src.host_runner_main:app --host 127.0.0.1 --port 8090

# Terminal 2:
cd ./arp
docker compose up --build

# Visit:
# http://localhost:8080/ (platform dashboard)
# http://localhost:8081/ (docs)
```


Platform endpoints:

- `http://127.0.0.1:8080/`
- `http://127.0.0.1:8080/api/health`
- `http://127.0.0.1:8080/api/rust/app/status`
- `POST http://127.0.0.1:8080/api/rust/app/compile`
- `POST http://127.0.0.1:8080/api/rust/app/start`
- `POST http://127.0.0.1:8080/api/rust/app/stop`

Rust app control notes:

- Compile uses `apps/arpsci` and writes to `apps/arpsci/target`.
- Start launches `apps/arpsci/target/debug/arpsci` with `ARPSCI_WEB_ENABLED=true`.

Dashboard static files:

- `platform/src/ui/index.html`
- `platform/src/ui/styles.css`
- `platform/src/ui/app.js`


</details>



## License

Proprietary