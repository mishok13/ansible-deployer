# AGENTS.md

## Commands
- Build: `cargo build` or `cargo b`
- Check (fast): `cargo check` or `cargo c`
- Run: `cargo run`
- Test all: `cargo test` or `cargo t`
- Test single: `cargo test test_name`
- Lint: `cargo clippy`
- Format: `cargo fmt`

## Architecture
- Simple Axum web server (edition 2024) listening on 127.0.0.1:3000
- Purpose: GitHub webhook receiver for releases (ansible deployment trigger)
- Single file application (src/main.rs)
- Uses tower-http for tracing middleware, serde for JSON deserialization
- Async runtime: tokio with multi-threaded runtime

## Code Style
- Standard Rust conventions (snake_case for functions/variables, PascalCase for types)
- Use serde's derive macros for JSON structures
- Prefer structured logging with tracing (use tracing::debug!, not println!)
- Extract custom types for request extractors (see ExtractUserAgent pattern)
- Use filter/map/ok_or chains for error handling (functional style)
- Keep imports grouped: external crates first, std library, then local modules
