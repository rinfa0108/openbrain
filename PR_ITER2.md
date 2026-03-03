## Iteration 2 — Query DSL + Structured Search (v0.1)

### Summary
- Implemented v0.1 Query DSL parsing into an AST (`openbrain-core`).
- Added structured search execution (`search_structured`) to the store API and Postgres implementation (`openbrain-store`).
- Implemented safe AST → SQL translation using **parameterized binds only** (no raw SQL passthrough), with a strict field whitelist and mandatory `scope` restriction.
- Added DB-backed tests for parsing correctness, execution correctness, and safety.

### Scope / Not in scope
- Not implemented: MCP server plumbing, HTTP endpoints, Query DSL over HTTP/MCP.
- Not implemented: semantic search / pgvector similarity, embeddings, normalization, provider integrations.
- Not implemented: promotion/conflict/timeline/policy engine.

### Supported DSL (v0.1)
- Comparisons: `==`, `!=`, `>`, `>=`, `<`, `<=`
- Membership: `IN [..]`
- Boolean logic: `AND`, `OR`, `NOT`
- Grouping: `( ... )`
- Field paths (whitelisted):
  - Top-level: `type`, `id`, `scope`, `status`, `spec_version`, `created_at`, `updated_at`, `tags`
  - Nested JSON: `data.<field>...` (multi-level via `data #>> '{path,subpath}'`)
  - Provenance: `provenance.ts`
- Regex `~=`: **disabled** in Iteration 2 (returns `OB_INVALID_REQUEST` with message).

### Examples
- `type == "claim" AND status IN ["draft","candidate"]`
- `data.meta.kind == "a" AND data.priority >= 5`
- `NOT (status == "deprecated" OR status == "superseded")`

### Safety guarantees
- Always restricts queries by `scope` (required).
- AST → SQL uses `sqlx` binds exclusively (`QueryBuilder` + `push_bind`) for values.
- Unknown/unsupported fields rejected with `OB_INVALID_REQUEST`.
- Injection-like inputs (e.g. semicolons / raw SQL fragments) rejected at parse time with `OB_INVALID_REQUEST`.

### DB schema / migrations
- Uses existing schema from `migrations/0001_init.sql` unchanged (no new migrations in Iteration 2).

### Tests
- Parser correctness: valid parse, invalid parse, precedence (`NOT > AND > OR`), regex disabled.
- Execution correctness: seeded objects queried by type/status/nested `data`.
- Safety: unknown field rejected; injection attempt rejected.
- DB strategy: tests apply migrations via `sqlx::migrate::Migrator` and use `DATABASE_URL` (CI provides Postgres+pgvector service).

### Quality gates (PASS)
- `cargo fmt --all -- --check` ✅
- `cargo clippy --all-targets --all-features -- -D warnings` ✅
- `cargo test --all --all-features` ✅

### Files changed
- `crates/openbrain-core/src/query.rs`
- `crates/openbrain-core/src/lib.rs`
- `crates/openbrain-store/src/lib.rs`
- `crates/openbrain-store/src/pg.rs`
- `crates/openbrain-store/tests/search_structured.rs`

### Follow-ups (Iteration 3+)
- Expose `search_structured` via MCP tools and/or HTTP API.
- Consider safe/guarded regex support if required.
- Add richer ordering, pagination metadata, and expanded response projection options.
