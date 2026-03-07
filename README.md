# OpenBrain (v0.1) — Provider-Agnostic Structured Memory for AI Agents

OpenBrain is an open-source **machine-readable memory layer** for agentic systems.  
It stores **typed, versioned memory objects** (claims, decisions, tasks, artifacts, entities, relations, thought summaries), supports **structured queries + semantic search**, and exposes a **standard plug-in protocol via MCP** (and a **mirror HTTP API**).

**Goal:** stop context switching / context rot by moving memory out of provider silos (ChatGPT/Codex/Claude/Gemini/local models) into an agent-owned infrastructure plane.

> **NOTE (Do not edit without explicit instruction):**  
> The **header + hero lines** above and the **License/footer** at the end of this README are considered **project identity text**.  
> Please **refrain from changing** them unless explicitly instructed by the project owner.

---

## Current status (implemented in main)

✅ Implemented (v0.1 core)
- Postgres storage for typed objects (`ob_objects`) + append-only event log (`ob_events`)
- Query DSL + deterministic structured search
- Embedding pipeline:
  - canonical text normalization + checksum (`sha256("ob.v0.1\n" + text)`)
  - embedding dedupe by `(scope, provider, model, kind, checksum)` (provider defaults to `noop`, kind to `semantic`)
  - `embed.generate`
- Semantic search using pgvector cosine ranking with optional safe DSL filters
- Local-first daemon:
  - MCP stdio: `openbrain mcp` (**primary for agents**)
  - HTTP: `openbrain serve` (**mirror/debug/SDK**)

🕒 Planned / not yet implemented
- MCP-over-HTTP transport
- Promotion workflow (`promote`), conflict detection (`conflicts.list`), timeline API, policy engine (`policy.explain`)
- Auth / multi-user / remote deployment defaults (currently local-only)
- Multi-dim embeddings (v0.2+)

---

## Quickstart

### Prerequisites
- Rust toolchain (stable) and Cargo
- Postgres with **pgvector** extension available
- A `DATABASE_URL` pointing to the Postgres instance

### Run Postgres + pgvector (example)
Use any Postgres that has pgvector. Example Docker image:
- `pgvector/pgvector:pg16`

(See `CONTRIBUTING.md` for concrete commands and curl examples.)

### Migrate
OpenBrain migrations live in `migrations/`.  
Tests run migrations automatically via `sqlx::migrate::Migrator`.

---

## Interfaces

### MCP stdio (primary for agents)
Run:
```bash
export DATABASE_URL="postgres://user:pass@localhost:5432/openbrain"
openbrain mcp
```

Implemented tools (names exact):
- `openbrain.ping`
- `openbrain.write`
- `openbrain.read` (scoped: `{ scope, refs }`)
- `openbrain.search.structured`
- `openbrain.embed.generate`
- `openbrain.search.semantic`
- `openbrain.rerank`
- `openbrain.memory.pack`

### HTTP mirror (debug/SDK)
Run:
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

Endpoints (POST):
- `/v1/ping`
- `/v1/write`
- `/v1/read` (scoped)
- `/v1/search/structured`
- `/v1/embed/generate`
- `/v1/search/semantic`
- `/v1/rerank`
- `/v1/memory/pack`

### Parity guarantee
MCP tools map **1:1** to the same store/service methods used by HTTP.  
Response envelopes and error codes are identical across both interfaces.

---

## SDK Quickstart (TS + Python)

SDKs live in:
- `sdk/typescript/openbrain-sdk`
- `sdk/python/openbrain_sdk`

Start the local server (HTTP examples):
```bash
export DATABASE_URL="postgres://user:pass@localhost:5432/openbrain"
export OPENBRAIN_EMBED_PROVIDER="fake"
openbrain serve
```

Run TypeScript examples:
```bash
cd sdk/typescript/openbrain-sdk
npm install
npx tsx examples/http_e2e.ts
npx tsx examples/mcp_e2e.ts
```

Run Python examples:
```bash
cd sdk/python/openbrain_sdk
py -3 -m pip install -e .
py -3 examples/http_e2e.py
py -3 examples/mcp_e2e.py
```

Notes:
- Examples run on localhost and do not require live API keys by default.
- `openbrain.memory.pack` requires `ANTHROPIC_API_KEY` set for the OpenBrain process.

---

## Embedding provider selection

OpenBrain uses a pluggable `EmbeddingProvider`.

Environment:
- `OPENBRAIN_EMBED_PROVIDER=noop` (default)  
- `OPENBRAIN_EMBED_PROVIDER=fake` (deterministic dev/test only)  
- `OPENBRAIN_EMBED_PROVIDER=openai` (real embeddings)
- `OPENBRAIN_EMBED_PROVIDER=local` (local HTTP embeddings)

OpenAI provider env (v0.1):
- `OPENAI_API_KEY` (required when provider=openai)
- `OPENAI_EMBED_MODEL` (optional; default: `text-embedding-3-small`)
- `OPENAI_BASE_URL` (optional)
- `OPENAI_TIMEOUT_SECS` (optional)
- `OPENAI_EMBED_DIMS` (optional; if set must be 1536)

> v0.1 uses a fixed embedding dimension of **1536** (pgvector column is `vector(1536)`).

Local HTTP provider env (v0.1):
- `LOCAL_EMBED_URL` (required when provider=local; example: `http://127.0.0.1:8080/embeddings`)
- `LOCAL_EMBED_MODEL` (optional)
- `LOCAL_EMBED_TIMEOUT_SECS` (optional)
- `LOCAL_EMBED_HEADER_*` (optional; forwarded as HTTP headers)

Local HTTP contract (implemented in v0.1):
Request:
```json
{ "model": "optional-model", "input": "text..." }
```

Response:
```json
{ "data": [ { "embedding": [0.01, 0.02, "..."] } ] }
```

Claude rerank/pack env (v0.1):
- `ANTHROPIC_API_KEY` (required for rerank/pack)
- `ANTHROPIC_MODEL` (optional)
- `ANTHROPIC_BASE_URL` (optional)
- `ANTHROPIC_TIMEOUT_SECS` (optional)

Claude is used **only** for rerank + memory pack summary. It is **not** an embedding provider.

---

## HTTP API (Implemented)

All endpoints are **POST** and accept/return JSON in a standard envelope.

Success:
```json
{ "ok": true, "...": "..." }
```

Error:
```json
{
  "ok": false,
  "error": { "code": "OB_INVALID_REQUEST", "message": "...", "details": {} }
}
```

### `POST /v1/ping`
Response:
```json
{ "ok": true, "version": "0.1", "server_time": "2026-03-03T10:00:00Z" }
```

### `POST /v1/write`
Request:
```json
{
  "objects": [
    {
      "type":"claim",
      "id":"clm_...",
      "scope":"workspace:nyex",
      "status":"draft",
      "spec_version":"0.1",
      "tags":[],
      "provenance":{ "actor":"agent:nyex", "ts":"..." },
      "data":{
        "subject":{ "entity_id":"ent_..." },
        "predicate":"requires",
        "object":{ "value":"MCP HTTP", "entity_id":null },
        "polarity":"affirm",
        "confidence":0.8,
        "evidence":[],
        "props":{}
      }
    }
  ],
  "mode":"draft",
  "idempotency_key":"optional"
}
```

### `POST /v1/read` (scoped)
Request:
```json
{ "scope": "workspace:nyex", "refs": ["clm_...", "dec_..."] }
```

### `POST /v1/search/structured`
Request:
```json
{
  "scope":"workspace:nyex",
  "where":"type == \"task\" AND data.state == \"blocked\"",
  "limit":50,
  "offset":0,
  "order_by":"updated_at DESC"
}
```

### `POST /v1/embed/generate`
Request (text):
```json
{ "scope":"workspace:nyex", "target": { "text":"routing budget policy" }, "model":"default" }
```

Request (ref):
```json
{ "scope":"workspace:nyex", "target": { "ref":"dec_..." }, "model":"default" }
```

### `POST /v1/search/semantic`
Request:
```json
{
  "scope":"workspace:nyex",
  "query":"routing budget policy",
  "top_k":10,
  "model":"default",
  "embedding_provider":"noop",
  "embedding_model":"default",
  "embedding_kind":"semantic",
  "filters":"status IN [\"candidate\",\"canonical\"]",
  "types":["decision","claim"],
  "status":["candidate","canonical"]
}
```

Response:
```json
{
  "ok": true,
  "matches": [
    { "ref":"dec_...", "kind":"decision", "score":0.83, "updated_at":"2026-03-03T10:05:00Z", "snippet": null }
  ]
}
```

---

## Query DSL (v0.1 implemented)

Supported operators:
- comparisons: `== != > >= < <=`
- membership: `IN [..]`
- boolean: `AND OR NOT`
- grouping: `( ... )`

Not implemented in v0.1:
- `~=` (regex match) — disabled (returns `OB_INVALID_REQUEST`)
- `CONTAINS` — not implemented

Field paths:
- top-level: `type`, `id`, `scope`, `status`, `spec_version`, `created_at`, `updated_at`, `tags`
- nested JSON: `data.<path>` (multi-level supported via JSON path)
- `provenance.ts`

---

## Storage schema (Postgres + pgvector)

OpenBrain uses:
- `ob_objects` — typed memory objects (JSONB `data` + `provenance`)
- `ob_events` — append-only events
- `ob_embeddings` — embeddings (`vector(1536)`), provider + model + kind

See `migrations/0001_init.sql` for exact DDL + indexes.

---

## Development

Local quality gates (source of truth):
```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all --all-features
```

DB tests:
- Use `DATABASE_URL`. If missing, DB-backed tests may skip with a clear message (see `CONTRIBUTING.md`).

---

## Roadmap (high level)

- IT7B: Claude rerank + Memory Pack Builder
- IT7C: Local embeddings provider
- IT8: Expand provider ecosystem + multi-embedding strategy (optional, later)

---

## License
Apache-2.0 (recommended for wide adoption) or MIT.

Digilabs Company Australia © NYEX AI Platform. All rights reserved. AIC Pty Ltd (ACN 082 378 256)
