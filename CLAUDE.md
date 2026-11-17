# CLAUDE.md

Phrasea report API: receives the events of the Phrasea applications (`POST /log`, sent by
`alchemy/report-sdk` in the phrasea repository) and writes them to PostgreSQL. A Rust
binary crate (`src/`), published as the `ps-report-api` image.

## Nothing is installed on the host — everything runs in Docker

- cargo: `bin/cargo.sh <args>` (Rust container = `rust-dev` stage of the Dockerfile, with rustfmt and clippy):
  `bin/cargo.sh test`, `bin/cargo.sh clippy --all-targets -- -D warnings`, `bin/cargo.sh fmt`.
- the image: `docker build -t phrasea-report .`

CI (`.github/workflows/ci.yaml`) runs fmt, clippy `-D warnings`, the unit tests, and
pushes the image to ECR Public.

## Layout

- `src/main.rs` (startup, shutdown), `config.rs` (environment), `http.rs` (`POST /log` → queue),
  `worker.rs` (single writer: insert, reconnect, replay), `store.rs` (`Store` trait, PostgreSQL implementation),
  `failed_logs.rs` (JSONL file of the refused entries), `model.rs` (`LogEntry`), `structure.sql` (schema).

## Conventions

- The HTTP contract (paths, status codes, bodies, accepted JSON) is used by the Phrasea applications: keep it stable.
- Tests: `#[cfg(test)]` modules next to the code; no test needs a running service (the worker is tested against a fake `Store`).
- Comments only for what a reader needs whatever the history (intent, constraints, pitfalls); never describe a change or a fix.
