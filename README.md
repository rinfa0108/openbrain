# OpenBrain (v0.1)

OpenBrain is a local-first, provider-agnostic **typed memory layer** for agent runtimes.

- **Primary agent integration:** **MCP (stdio)** via `openbrain mcp`
- **Mirror/debug/SDK interface:** **HTTP** via `openbrain serve` (`/v1/*` on localhost)

OpenBrain provides:

- Typed, versioned memory objects stored in Postgres (`ob_objects`) + append-only event log (`ob_events`)
- Deterministic structured search using a safe Query DSL
- Embedding pipeline + semantic search via pgvector (cosine similarity)

## Quickstart

### Prerequisites

- Rust (stable)
- Postgres with `pgvector` available
- `DATABASE_URL` set (required)

Migrations live in `migrations/`.

### Run MCP stdio (primary)

```bash
export DATABASE_URL="postgres://user:pass@localhost:5432/openbrain"
openbrain mcp
```

### Run HTTP daemon (mirror)

```bash
export DATABASE_URL="postgres://user:pass@localhost:5432/openbrain"
openbrain serve
```

Defaults:

- Bind: `127.0.0.1`
- Port: `7981`

Overrides:

- `OPENBRAIN_BIND`
- `OPENBRAIN_PORT`

### Embedding provider selection

Environment:

- `OPENBRAIN_EMBED_PROVIDER=noop` (default)
  - `embed.generate` / semantic search will return `OB_EMBEDDING_FAILED` with a clear message
- `OPENBRAIN_EMBED_PROVIDER=fake` (dev/testing only)
  - deterministic 1536-dim embeddings

> v0.1 uses fixed dims `1536` (pgvector column is `vector(1536)`).

## Envelopes + parity

All interfaces use the same envelope:

- success: `{ "ok": true, ... }`
- error: `{ "ok": false, "error": { "code": "...", "message": "...", "details": { ... } } }`

**Parity statement (required):** MCP tools map **1:1** to the same store/service methods used by HTTP. Envelopes and canonical error codes are identical across both interfaces.

## MCP tools (implemented)

Tool names (exact):

- `openbrain.ping`
- `openbrain.write`
- `openbrain.read` (**scoped**: requires `scope` + `refs`)
- `openbrain.search.structured`
- `openbrain.embed.generate`
- `openbrain.search.semantic`

Notes:

- Tools that require scope will return `OB_SCOPE_REQUIRED` if `scope` is missing/blank.

Request argument shapes (high level):

- `openbrain.ping` arguments: `{}`
- `openbrain.write` arguments: `PutObjectsRequest` (see HTTP `/v1/write`)
- `openbrain.read` arguments: `{ "scope": "...", "refs": ["..."] }`
- `openbrain.search.structured` arguments: `SearchStructuredRequest`
- `openbrain.embed.generate` arguments: `EmbedGenerateRequest`
- `openbrain.search.semantic` arguments: `SearchSemanticRequest`

## HTTP endpoints (implemented mirror)

Endpoints (exact, all **POST**):

- `/v1/ping`
- `/v1/write`
- `/v1/read` (**scoped**)
- `/v1/search/structured`
- `/v1/embed/generate`
- `/v1/search/semantic`

Examples:

- Ping:
  - `curl -sS -X POST http://127.0.0.1:7981/v1/ping`

## Query DSL (implemented subset, v0.1)

Supported:

- Comparisons: `==`, `!=`, `>`, `>=`, `<`, `<=`
- Membership: `IN [..]`
- Boolean: `AND`, `OR`, `NOT`
- Grouping: `( ... )`

Field paths:

- Top-level fields: `type`, `id`, `scope`, `status`, `spec_version`, `created_at`, `updated_at`, `tags`
- Nested JSON: `data.<field>` and `data.<field>.<subfield>`
- `provenance.ts`

Not implemented (explicit):

- Regex operator `~=`
- `CONTAINS`

Invalid syntax returns `OB_INVALID_REQUEST` (with line/col details when available).

## Not implemented (explicit)

These items are **not implemented** (no ambiguity):

- `promote`, `conflicts.list`, timeline APIs, `policy.explain`
- Auth, remote bind defaults (daemon is local-first)
- Multi-dim embeddings
- MCP-over-HTTP transport (stdio only)

## Dev

See `CONTRIBUTING.md` for:

- `cargo fmt` / `cargo clippy` / `cargo test`
- Postgres + pgvector setup
- HTTP curl examples
