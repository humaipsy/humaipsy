@echo off
setlocal

set "ROOT_DIR=%~dp0..\"

cd /d "%ROOT_DIR%apps\humaipsy" || exit /b 1
cargo clean || exit /b 1
cargo run
