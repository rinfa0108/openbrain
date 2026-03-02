# OpenBrain (v0.1) — Provider-Agnostic Structured Memory for AI Agents

OpenBrain is an open-source **machine-readable memory layer** for agentic systems.  
It stores **typed, versioned memory objects** (claims, decisions, tasks, artifacts, entities, relations, thought summaries), supports **structured queries + semantic search**, and exposes a **standard plug-in protocol** via **MCP** (and a mirror HTTP API).

> Goal: stop context switching / context rot by moving memory out of provider silos (ChatGPT/Codex/Claude/Gemini/local models) into an agent-owned infrastructure plane.

---

## Contents

- [1. Concepts](#1-concepts)
- [2. OpenBrain v0.1 API Spec](#2-openbrain-v01-api-spec)
  - [2.1 Tool List (MCP)](#21-tool-list-mcp)
  - [2.2 Error Codes](#22-error-codes)
  - [2.3 JSON Schemas](#23-json-schemas)
- [3. Query DSL (v0.1)](#3-query-dsl-v01)
- [4. Minimal Postgres Schema + Indexes](#4-minimal-postgres-schema--indexes)
- [5. Embedding Normalization Rules](#5-embedding-normalization-rules)
- [6. Conformance Tests Outline](#6-conformance-tests-outline)
- [7. Build & Runtime Architecture](#7-build--runtime-architecture)
- [8. Roadmap (v0.2+)](#8-roadmap-v02)
- [License](#license)

---

## 1. Concepts

OpenBrain is **not** “chat history storage”. It is typed memory infrastructure.

### Core Object Types (v0.1)

- **entity**: node (project/system/person/concept)
- **relation**: typed edge between entities
- **claim**: atomic statement with confidence/status/evidence
- **decision**: chosen outcome with rationale/options
- **task**: plan state, steps, status
- **artifact**: pointer to external content (file/url/snippet) with checksum
- **thought_summary**: structured reasoning summary (safe-by-default)

### Status & anti-rot

Objects can be: `draft | candidate | canonical | deprecated | superseded`  
Promotion to canonical should be gated (policy/tool verification/user approval).

---

## 2. OpenBrain v0.1 API Spec

OpenBrain exposes a **standard contract** through MCP tools and mirrors the same endpoints through HTTP (`/v1/...`) for non-MCP runtimes.

### 2.1 Tool List (MCP)

All MCP tools return either:

- success: `{"ok": true, ...}`
- error: `{"ok": false, "error": { "code": "...", "message": "...", "details": {...} }}`

#### `openbrain.ping() -> { ok: true, version, server_time }`

Health check / capability probe.

**Response**
```json
{
  "ok": true,
  "version": "0.1",
  "server_time": "2026-03-03T10:00:00Z"
}
```

#### `openbrain.write(request) -> response`

Write one or many objects (draft or canonical).

**Request**
- `objects[]` (typed objects)
- `mode`: `"draft" | "canonical"` (canonical may be rejected by policy)
- `idempotency_key` (optional)

```json
{
  "objects": [ { "type": "claim", "id": "clm_...", "scope": "workspace:nyex", "status": "draft", "spec_version": "0.1", "tags": [], "provenance": { "actor": "agent:nyex", "ts": "2026-03-03T10:00:00Z", "source": "mcp" }, "data": { "subject": { "entity_id": "ent_..." }, "predicate": "requires", "object": { "value": "MCP HTTP transport", "entity_id": null }, "polarity": "affirm", "confidence": 0.78, "evidence": [], "props": {} } } ],
  "mode": "draft",
  "idempotency_key": "idem_123"
}
```

**Response**
```json
{
  "ok": true,
  "results": [
    {
      "ref": "clm_...",
      "status": "stored",
      "version": 1,
      "warnings": []
    }
  ]
}
```

#### `openbrain.read(request) -> response`

Read objects by reference.

**Request**
```json
{ "refs": ["clm_...", "dec_..."] }
```

**Response**
```json
{
  "ok": true,
  "objects": [ /* full objects */ ]
}
```

#### `openbrain.search.semantic(request) -> response`

Semantic search over embeddings.

**Request**
- `query`: string
- `scope`: string
- `top_k`: number (default 10)
- `filters`: optional Query DSL subset

```json
{
  "query": "routing budget policy",
  "scope": "workspace:nyex",
  "top_k": 10,
  "filters": "status IN [\"candidate\",\"canonical\"]"
}
```

**Response**
```json
{
  "ok": true,
  "matches": [
    { "ref": "dec_...", "kind": "decision", "score": 0.83, "snippet": "Use MCP as primary...", "updated_at": "2026-03-03T10:05:00Z" }
  ]
}
```

#### `openbrain.search.structured(request) -> response`

Deterministic query using Query DSL.

**Request**
```json
{
  "scope": "workspace:nyex",
  "where": "type == \"task\" AND data.state == \"blocked\"",
  "limit": 50,
  "offset": 0,
  "order_by": "updated_at DESC"
}
```

**Response**
```json
{
  "ok": true,
  "results": [
    { "ref": "tsk_...", "type": "task", "status": "candidate", "updated_at": "2026-03-03T10:10:00Z" }
  ]
}
```

#### `openbrain.embed.generate(request) -> response`

Generate embeddings for raw text or existing objects.

**Request**
```json
{
  "target": { "ref": "dec_..." },
  "model": "text-embedding-3-large",
  "dims": 1536
}
```

**Response**
```json
{
  "ok": true,
  "embedding_id": "emb_...",
  "object_ref": "dec_...",
  "checksum": "sha256:...",
  "dims": 1536,
  "model": "text-embedding-3-large"
}
```

#### `openbrain.promote(request) -> response`

Promote a draft/candidate object.

**Request**
```json
{
  "ref": "clm_...",
  "to_status": "canonical",
  "reason": "Validated by test suite output"
}
```

**Response**
```json
{
  "ok": true,
  "ref": "clm_...",
  "old_status": "candidate",
  "new_status": "canonical"
}
```

#### `openbrain.conflicts.list(request) -> response`

List detected conflicts in canonical memory.

**Request**
```json
{ "scope": "workspace:nyex", "limit": 100 }
```

**Response**
```json
{
  "ok": true,
  "conflicts": [
    {
      "conflict_id": "cfl_...",
      "kind": "claim_contradiction",
      "refs": ["clm_a", "clm_b"],
      "summary": "Two canonical claims contradict on predicate=requires"
    }
  ]
}
```

#### `openbrain.timeline(request) -> response`

Read append-only event log.

**Request**
```json
{ "scope": "workspace:nyex", "since": "2026-03-01T00:00:00Z", "limit": 200 }
```

**Response**
```json
{
  "ok": true,
  "events": [
    { "id": 1, "event_type": "object_written", "actor": "agent:nyex", "ts": "2026-03-03T10:00:00Z", "payload": { "ref": "clm_..." } }
  ]
}
```

#### `openbrain.policy.explain(request) -> response`

Explain why an action would be blocked (or allowed).

**Request**
```json
{
  "scope": "workspace:nyex",
  "action": "promote",
  "object": { "type": "claim", "id": "clm_...", "status": "draft" }
}
```

**Response**
```json
{
  "ok": true,
  "allowed": false,
  "reasons": ["canonical promotion requires user approval or tool verification evidence"],
  "required_conditions": ["evidence.kind == \"artifact\" AND evidence.quote != \"\""]
}
```

---

### 2.2 Error Codes

All errors are returned in a standard envelope:

```json
{
  "ok": false,
  "error": {
    "code": "OB_INVALID_SCHEMA",
    "message": "Object failed schema validation",
    "details": { "path": "objects[0].data.predicate" }
  }
}
```

#### Canonical error codes (v0.1)

- `OB_INVALID_REQUEST` — malformed request payload
- `OB_INVALID_SCHEMA` — JSON schema validation failed
- `OB_UNSUPPORTED_VERSION` — client/server spec mismatch
- `OB_SCOPE_REQUIRED` — scope missing or invalid
- `OB_NOT_FOUND` — referenced object does not exist
- `OB_CONFLICT` — optimistic concurrency/version conflict
- `OB_POLICY_DENIED` — policy engine denied write/promote/read
- `OB_EMBEDDING_FAILED` — embedding generation error
- `OB_STORAGE_ERROR` — DB failure
- `OB_RATE_LIMITED` — throttled
- `OB_INTERNAL` — unexpected server error

---

### 2.3 JSON Schemas

OpenBrain uses **typed objects** with a shared envelope. These are *normative* v0.1 shapes.

#### `MemoryObject` (base)

```json
{
  "type": "claim",
  "id": "clm_01J....",
  "scope": "workspace:nyex",
  "status": "draft",
  "tags": ["routing", "budget"],
  "provenance": {
    "actor": "agent:nyex",
    "ts": "2026-03-03T10:00:00Z",
    "source": "mcp",
    "trace_id": "trc_..."
  },
  "spec_version": "0.1",
  "data": {}
}
```

#### Schema: `Entity`

```json
{
  "type": "entity",
  "id": "ent_...",
  "scope": "workspace:default",
  "status": "canonical",
  "spec_version": "0.1",
  "tags": [],
  "provenance": { "actor": "agent:nyex", "ts": "2026-03-03T10:00:00Z", "source": "mcp" },
  "data": {
    "entity_type": "project",
    "name": "NYEX",
    "props": { "repo": "rinfa0108/nyex" }
  }
}
```

#### Schema: `Relation`

```json
{
  "type": "relation",
  "id": "rel_...",
  "scope": "workspace:default",
  "status": "canonical",
  "spec_version": "0.1",
  "tags": [],
  "provenance": { "actor": "agent:nyex", "ts": "2026-03-03T10:00:00Z", "source": "mcp" },
  "data": {
    "src_entity_id": "ent_...",
    "rel_type": "depends_on",
    "dst_entity_id": "ent_...",
    "props": {}
  }
}
```

#### Schema: `Claim`

```json
{
  "type": "claim",
  "id": "clm_...",
  "scope": "workspace:default",
  "status": "draft",
  "spec_version": "0.1",
  "tags": [],
  "provenance": { "actor": "agent:nyex", "ts": "2026-03-03T10:00:00Z", "source": "mcp" },
  "data": {
    "subject": { "entity_id": "ent_..." },
    "predicate": "requires",
    "object": { "value": "MCP HTTP transport", "entity_id": null },
    "polarity": "affirm",
    "confidence": 0.78,
    "evidence": [
      { "kind": "artifact", "ref": "art_...", "quote": "Implemented MCP HTTP transport" }
    ],
    "props": {}
  }
}
```

#### Schema: `Decision`

```json
{
  "type": "decision",
  "id": "dec_...",
  "scope": "workspace:default",
  "status": "canonical",
  "spec_version": "0.1",
  "tags": ["protocol"],
  "provenance": { "actor": "user:richard", "ts": "2026-03-03T10:05:00Z", "source": "mcp" },
  "data": {
    "title": "Use MCP as primary tool protocol",
    "outcome": "A",
    "options": [
      { "id": "A", "text": "MCP first" },
      { "id": "B", "text": "HTTP first" }
    ],
    "rationale": "Immediate compatibility; keep REST mirror for non-MCP clients.",
    "props": {}
  }
}
```

#### Schema: `Task`

```json
{
  "type": "task",
  "id": "tsk_...",
  "scope": "workspace:default",
  "status": "candidate",
  "spec_version": "0.1",
  "tags": ["implementation"],
  "provenance": { "actor": "agent:nyex", "ts": "2026-03-03T10:10:00Z", "source": "mcp" },
  "data": {
    "title": "Implement embedding cache",
    "state": "in_progress",
    "steps": [
      { "text": "Add checksum column", "status": "done" },
      { "text": "Add reuse lookup by checksum", "status": "todo" }
    ],
    "props": {}
  }
}
```

#### Schema: `Artifact`

```json
{
  "type": "artifact",
  "id": "art_...",
  "scope": "workspace:default",
  "status": "canonical",
  "spec_version": "0.1",
  "tags": [],
  "provenance": { "actor": "agent:nyex", "ts": "2026-03-03T10:00:00Z", "source": "mcp" },
  "data": {
    "kind": "url",
    "uri": "https://example.com/spec",
    "checksum": "sha256:...",
    "metadata": { "mime": "text/html" }
  }
}
```

#### Schema: `ThoughtSummary`

```json
{
  "type": "thought_summary",
  "id": "tht_...",
  "scope": "workspace:default",
  "status": "draft",
  "spec_version": "0.1",
  "tags": ["design"],
  "provenance": { "actor": "agent:nyex", "ts": "2026-03-03T10:00:00Z", "source": "mcp" },
  "data": {
    "intent": "Design memory layer",
    "assumptions": ["Providers are siloed"],
    "constraints": ["No secrets in canonical store"],
    "open_questions": ["Default embedding model?"],
    "next_actions": ["Draft MCP tool spec", "Create migrations", "Implement SDK"],
    "props": {}
  }
}
```

---

## 3. Query DSL (v0.1)

OpenBrain Query DSL is a safe, deterministic filter language for structured search.

### 3.1 Grammar (informal)

- Comparisons: `field == value`, `field != value`, `field ~= "regex"`, `field IN [a,b]`
- Boolean: `AND`, `OR`, `NOT`
- Grouping: `( ... )`
- Values: strings `"..."`, numbers, booleans, null, arrays `[ ... ]`
- Fields:
  - top-level: `type`, `id`, `scope`, `status`, `tags`, `spec_version`, `created_at`, `updated_at`
  - nested: `data.title`, `data.entity_type`, `data.predicate`, `data.state`, `provenance.ts`

### 3.2 Operators

- `==`, `!=`
- `>`, `>=`, `<`, `<=` (numeric/date)
- `IN` (membership)
- `~=` (regex match, RE2 subset)
- `CONTAINS` (array contains value)

### 3.3 Examples

- Find canonical decisions in a workspace:
  - `type == "decision" AND status == "canonical" AND scope == "workspace:nyex"`
- Find blocked tasks:
  - `type == "task" AND data.state == "blocked"`
- Find claims with predicate `requires`:
  - `type == "claim" AND data.predicate == "requires" AND status IN ["candidate","canonical"]`
- Find artifacts by checksum:
  - `type == "artifact" AND data.checksum == "sha256:..."`

### 3.4 Safety Rules

- Max expression length (server config)
- Disallow SQL/functions
- Regex limited and time-bounded
- Field whitelist per object type

---

## 4. Minimal Postgres Schema + Indexes

### 4.1 Required extensions

```sql
CREATE EXTENSION IF NOT EXISTS pgcrypto;
CREATE EXTENSION IF NOT EXISTS vector; -- pgvector
```

### 4.2 Tables (minimal)

```sql
-- Single table for all typed memory objects
CREATE TABLE IF NOT EXISTS ob_objects (
  id            TEXT PRIMARY KEY,
  scope         TEXT NOT NULL,
  type          TEXT NOT NULL,
  status        TEXT NOT NULL,
  spec_version  TEXT NOT NULL DEFAULT '0.1',
  tags          TEXT[] NOT NULL DEFAULT '{}',
  data          JSONB NOT NULL,
  provenance    JSONB NOT NULL,
  version       BIGINT NOT NULL DEFAULT 1,
  created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Append-only event log
CREATE TABLE IF NOT EXISTS ob_events (
  id          BIGSERIAL PRIMARY KEY,
  scope       TEXT NOT NULL,
  event_type  TEXT NOT NULL,
  actor       TEXT NOT NULL,
  payload     JSONB NOT NULL,
  ts          TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Embeddings table
-- NOTE: pgvector requires fixed dimension; v0.1 recommends standardizing dims (e.g. 1536).
CREATE TABLE IF NOT EXISTS ob_embeddings (
  id            TEXT PRIMARY KEY,
  object_id     TEXT NULL REFERENCES ob_objects(id) ON DELETE CASCADE,
  scope         TEXT NOT NULL,
  model         TEXT NOT NULL,
  dims          INT  NOT NULL,
  checksum      TEXT NOT NULL, -- checksum of normalized text used for embedding
  embedding     vector(1536),
  created_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);
```

### 4.3 Indexes

```sql
-- common filters
CREATE INDEX IF NOT EXISTS ob_objects_scope_type_status_idx
  ON ob_objects (scope, type, status);

CREATE INDEX IF NOT EXISTS ob_objects_updated_idx
  ON ob_objects (scope, updated_at DESC);

-- JSONB query support (basic)
CREATE INDEX IF NOT EXISTS ob_objects_data_gin
  ON ob_objects USING GIN (data);

-- tags
CREATE INDEX IF NOT EXISTS ob_objects_tags_gin
  ON ob_objects USING GIN (tags);

-- event timeline
CREATE INDEX IF NOT EXISTS ob_events_scope_ts_idx
  ON ob_events (scope, ts DESC);

-- embeddings lookup + dedupe
CREATE INDEX IF NOT EXISTS ob_embeddings_scope_model_checksum_idx
  ON ob_embeddings (scope, model, checksum);

-- vector index (pgvector)
CREATE INDEX IF NOT EXISTS ob_embeddings_vec_idx
  ON ob_embeddings USING ivfflat (embedding vector_cosine_ops) WITH (lists = 100);
```

---

## 5. Embedding Normalization Rules

To prevent embedding drift, OpenBrain MUST embed a **canonical text form**.

### 5.1 Normalization pipeline (v0.1)

1) **Select embedding text** per object type:
   - entity: `ENTITY: {entity_type} | {name} | props:{sorted props json}`
   - claim: `CLAIM: subj:{subject} pred:{predicate} obj:{object} pol:{polarity}`
   - decision: `DECISION: {title} outcome:{outcome} rationale:{rationale}`
   - task: `TASK: {title} state:{state} steps:{joined step texts}`
   - artifact: `ARTIFACT: {kind} uri:{uri} checksum:{checksum}`
   - thought_summary: `THOUGHT: intent:{intent} assumptions:{...} constraints:{...} actions:{...}`

2) **Stable JSON serialization** for included `props`:
   - sort object keys recursively
   - remove null fields
   - normalize whitespace to single spaces
   - convert CRLF -> LF
   - trim ends

3) **Checksum**:
   - `checksum = sha256("ob.v0.1\n" + normalized_text)`
   - store as `sha256:<hex>`

4) **Idempotent embeddings**:
   - if `(scope, model, checksum)` exists → reuse embedding
   - else generate and store

### 5.2 What NOT to embed

- secrets (API keys, tokens)
- raw chain-of-thought unless explicitly enabled and encrypted
- large binary content (embed extracted text only)

---

## 6. Conformance Tests Outline

OpenBrain should ship with a test suite that validates interoperability.

### 6.1 Schema Conformance

- validate each object type against JSON schema
- ensure required fields present
- reject unknown top-level `type`s

### 6.2 Protocol Conformance

- MCP tool names and inputs match spec
- error envelope format and codes correct
- pagination (`limit/offset`) behavior stable

### 6.3 Storage Conformance

- write then read returns identical object (except server-managed fields)
- version increments correctly on update
- events append for every write/promote

### 6.4 Query DSL Conformance

- operator correctness (`==`, `IN`, `~=`, `AND/OR/NOT`)
- field whitelist enforced
- invalid expressions return `OB_INVALID_REQUEST`

### 6.5 Embedding Conformance

- normalization stable across OS (LF normalization)
- checksum reproducible
- embedding reused when checksum unchanged
- rejects embedding requests for disallowed content

### 6.6 Semantic Search Conformance

- semantic search returns deterministic envelope + sorted by score
- filters apply post-retrieval consistently
- `top_k` respected

---

## 7. Build & Runtime Architecture

OpenBrain runs as a **local-first service**:

- MCP server (stdio and/or MCP-over-HTTP transport)
- HTTP API (optional mirror)
- Postgres backend (local container, remote, or managed)
- Embedding provider(s): OpenAI / local models / pluggable providers

### Recommended v0.1 runtime

- `openbrain` daemon on localhost
  - HTTP: `127.0.0.1:7981`
  - MCP: stdio (for local agent runners) and/or HTTP (for remote runners)
- Postgres: local Docker or remote managed

---

## 8. Roadmap (v0.2+)

- Multi-dim embedding tables (or multiple vector columns)
- Strong policy engine (RBAC/ABAC)
- Conflict detection rules per type (claim contradictions)
- Encryption-at-rest for sensitive memory objects
- Replication and multi-node sync
- **Memory packs** (curated context bundles per agent/task)

---

## License

Apache-2.0 (recommended for wide adoption) or MIT.

Digilabs Company Australia © NYEX AI Platform. All rights reserved.
AIC Pty Ltd (ACN 082 378 256)
