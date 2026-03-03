# Contributing

## Prereqs

- Rust (stable): install via `rustup`
- (Optional) Postgres + `pgvector` for local DB work

## Developer commands

From the repo root:

- Format: `cargo fmt --all -- --check`
- Lints: `cargo clippy --all-targets --all-features -- -D warnings`
- Tests: `cargo test --all --all-features`

## Migrations (placeholder)

Iteration 0 provides SQL migrations only.

- Apply locally (example): `psql "$DATABASE_URL" -f migrations/0001_init.sql`
