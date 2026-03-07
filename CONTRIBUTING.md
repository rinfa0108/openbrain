# Contributing

## Prereqs

- Rust (stable): install via `rustup`
- Postgres + `pgvector` (required for storage + search tests)

## Developer commands

From the repo root:

- Format: `cargo fmt --all -- --check`
- Lints: `cargo clippy --all-targets --all-features -- -D warnings`
- Tests: `cargo test --all --all-features`

## Quality gates (security + code quality)

Run the full local gate script from the repo root:

- PowerShell: `scripts/ci/quality-gates.ps1`
- Bash: `scripts/ci/quality-gates.sh`

Tooling installs:

- cargo-deny: `cargo install cargo-deny`
- gitleaks (choco): `choco install gitleaks`
- gitleaks (brew): `brew install gitleaks`
- gitleaks (scoop): `scoop install gitleaks`

Allowlists and policy live in:

- Dependency policy: `deny.toml`
- Secret scan config: `.gitleaks.toml`

## Migrations

Iteration 0 provides SQL migrations.

- Apply locally (example): `psql "$DATABASE_URL" -f migrations/0001_init.sql`

## Postgres for tests

Storage/search tests expect `DATABASE_URL` to point at a Postgres instance with `pgvector` available.

Example (Docker):

- Start DB: `docker run --rm -p 5432:5432 -e POSTGRES_PASSWORD=postgres pgvector/pgvector:pg16`
- Set DSN: `DATABASE_URL=postgres://postgres:postgres@localhost:5432/postgres`

## Daemon (Iteration 5)

### Run the server

- Required env:
  - `DATABASE_URL=postgres://...`
- Optional env:
  - `OPENBRAIN_BIND=127.0.0.1` (default)
  - `OPENBRAIN_PORT=7981` (default)
  - `OPENBRAIN_EMBED_PROVIDER=noop|fake|openai|local` (default: `noop`)

Run from repo root:

- `cargo run -p openbrain-server -- serve`

Notes:

- The daemon binds to localhost by default.
- Embeddings:
  - Default provider is `noop` (no external network/API keys required). `/v1/embed/generate` and `/v1/search/semantic` will return `OB_EMBEDDING_FAILED` with a clear message.
  - `fake` is available for local dev/testing only (explicitly opt-in via `OPENBRAIN_EMBED_PROVIDER=fake`).

### HTTP API (JSON envelopes)

All endpoints are `POST` and return the standard envelope:

- success: `{ "ok": true, ... }`
- error: `{ "ok": false, "error": { "code": "...", "message": "...", "details": {...} } }`

Minimal examples (adjust `scope`/`id`):

- Ping:
  - `curl -sS -X POST http://127.0.0.1:7981/v1/ping`

- Write:
  - `curl -sS -X POST http://127.0.0.1:7981/v1/write -H "content-type: application/json" -d '{"objects":[{"type":"claim","id":"c1","scope":"demo","status":"draft","spec_version":"0.1","tags":[],"data":{"subject":"a","predicate":"b","object":"c","polarity":"pos"},"provenance":{"actor":"me"}}]}'`

- Read (scoped):
  - `curl -sS -X POST http://127.0.0.1:7981/v1/read -H "content-type: application/json" -d '{"scope":"demo","refs":["c1"]}'`

- Structured search:
  - `curl -sS -X POST http://127.0.0.1:7981/v1/search/structured -H "content-type: application/json" -d '{"scope":"demo","where_expr":"type == \"claim\"","limit":10,"offset":0}'`

- Embed generate:
  - `curl -sS -X POST http://127.0.0.1:7981/v1/embed/generate -H "content-type: application/json" -d '{"scope":"demo","target":{"ref":"c1"},"model":"fake","dims":1536}'`

- Semantic search:
  - `curl -sS -X POST http://127.0.0.1:7981/v1/search/semantic -H "content-type: application/json" -d '{"scope":"demo","query":"a b c","top_k":5,"model":"fake"}'`

## MCP (Iteration 6)

### Run MCP stdio

- Required env:
  - `DATABASE_URL=postgres://...`
- Optional env:
  - `OPENBRAIN_EMBED_PROVIDER=noop|fake|openai|local` (default: `noop`)

Run from repo root:

- `cargo run -p openbrain-server -- mcp`

### Quick MCP smoke test (high level)

Use any MCP-capable client/host and call:

- tool: `openbrain.ping`
- arguments: `{}`

Expected envelope:

- `{ "ok": true, "version": "0.1", "server_time": "..." }`

## OpenAI embeddings provider (IT7A)

Enable real embeddings (no paid CI required; local-only):

- `OPENBRAIN_EMBED_PROVIDER=openai`
- `OPENAI_API_KEY=...` (required)

Optional:

- `OPENAI_EMBED_MODEL` (default: `text-embedding-3-small`)
- `OPENAI_BASE_URL` (default: `https://api.openai.com`)
- `OPENAI_TIMEOUT_SECS` (default: `30`)
- `OPENAI_EMBED_DIMS` (optional; if set must be `1536`)

Live OpenAI tests are opt-in only:

- set `RUN_OPENAI_LIVE_TESTS=1` and `OPENAI_API_KEY` to run `crates/openbrain-embed/tests/openai_live.rs`

## Local HTTP embeddings provider (IT7C)

Enable local embeddings without external keys:

- `OPENBRAIN_EMBED_PROVIDER=local`
- `LOCAL_EMBED_URL=http://127.0.0.1:8080/embeddings` (required)

Optional:

- `LOCAL_EMBED_MODEL`
- `LOCAL_EMBED_TIMEOUT_SECS`
- `LOCAL_EMBED_HEADER_*` (e.g., `LOCAL_EMBED_HEADER_AUTHORIZATION=Bearer ...`)

Expected JSON contract (v0.1):

Request:
```json
{ "model": "optional-model", "input": "text..." }
```

Response:
```json
{ "data": [ { "embedding": [0.01, 0.02, "..."] } ] }
```

## Claude rerank + memory pack (IT7B)

Required env:

- `ANTHROPIC_API_KEY=...`

Optional:

- `ANTHROPIC_MODEL` (default: `claude-3-5-sonnet-latest`)
- `ANTHROPIC_BASE_URL`
- `ANTHROPIC_TIMEOUT_SECS`

Endpoints/tools added:

- HTTP: `POST /v1/rerank`, `POST /v1/memory/pack`
- MCP: `openbrain.rerank`, `openbrain.memory.pack`

Live Anthropic tests are opt-in only:

- set `RUN_ANTHROPIC_LIVE_TESTS=1` and `ANTHROPIC_API_KEY` to run `crates/openbrain-llm/tests/anthropic_live.rs`
