# ARPSCI Ecosystem

Under heavy development. 

Repository for the ARPSCI ecosystem.

## Dependencies

- Git (>= 2.30)
- Docker Engine (>= 24.0)
- Docker Compose plugin (>= 2.20, Compose V2)
- Rust toolchain (stable with Edition 2024 support; rustc >= 1.85)
- Cargo (bundled with Rust toolchain)
- Python (>= 3.12)
- uv (latest stable)



## Reproduce

```sh
# ARPSCI and Platform
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



## Rust Application Development

Use these commands when you want to develop only the Rust app.
The app has its own `.cargo/config.toml` that keeps `target/` local to `apps/arpsci/`,
so you work entirely inside that directory.

```sh
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
```

Useful during iteration:

```sh
# Run tests
cargo test

# Format and lint (if needed before commit)
cargo fmt
cargo clippy --all-targets -- -D warnings
```


## Python platform

```sh
cd platform
uv sync --dev
uv run uvicorn src.main:app --reload --host 127.0.0.1 --port 8080
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

