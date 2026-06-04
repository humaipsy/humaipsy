# Humai v.0.1

(Under heavy development)

Features:

- Cross-platform Application for Human-AI Interaction
- Cognitive Science and HCI Experimental Design Application 
- Integrated with Open-Weight Models
- Local and Offline-First Field Architecture
- Statistical Analysis Friendly Data Output

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
- Rust toolchain (stable with Edition 2024 support; rustc >= 1.85)
- Cargo (bundled with Rust toolchain)


## Reproduce

Development 

<details>
<summary>Commands</summary>
<br>

```sh
# Rust Application Development
./run_app.sh

# Manually
cd ./apps/arpsci

# Fast compile check (all bins and tests)
cargo check --all-targets

# Run app (UI only)
cargo run

# Run the helper binary to create/update runs/.master_hash (run from repo root)
cargo run --bin gen_master_hash

# Build release binary
cargo build --release

# Run tests
cargo test

# Format and lint (if needed before commit)
cargo fmt
cargo clippy --all-targets -- -D warnings

```


</details>



## License

Proprietary