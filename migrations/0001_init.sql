-- OpenBrain minimal schema (v0.1)

CREATE EXTENSION IF NOT EXISTS pgcrypto;
CREATE EXTENSION IF NOT EXISTS vector; -- pgvector

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
