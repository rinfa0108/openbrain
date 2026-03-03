# Contributing

## Prereqs

- Rust (stable): install via `rustup`
- Postgres + `pgvector` (required for storage tests)

## Developer commands

From the repo root:

- Format: `cargo fmt --all -- --check`
- Lints: `cargo clippy --all-targets --all-features -- -D warnings`
- Tests: `cargo test --all --all-features`

## Migrations

Iteration 0 provides SQL migrations.

- Apply locally (example): `psql "$DATABASE_URL" -f migrations/0001_init.sql`

## Postgres for tests

Storage tests expect `DATABASE_URL` to point at a Postgres instance with `pgvector` available.

Example (Docker):

- Start DB: `docker run --rm -p 5432:5432 -e POSTGRES_PASSWORD=postgres pgvector/pgvector:pg16`
- Set DSN: `DATABASE_URL=postgres://postgres:postgres@localhost:5432/postgres`
