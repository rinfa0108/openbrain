-- Multi-embedding strategy (v0.1 -> v0.1+)

ALTER TABLE ob_embeddings
  ADD COLUMN IF NOT EXISTS provider TEXT NOT NULL DEFAULT 'noop';

ALTER TABLE ob_embeddings
  ADD COLUMN IF NOT EXISTS kind TEXT NOT NULL DEFAULT 'semantic';

UPDATE ob_embeddings
  SET provider = 'noop'
  WHERE provider IS NULL;

UPDATE ob_embeddings
  SET kind = 'semantic'
  WHERE kind IS NULL;

DROP INDEX IF EXISTS ob_embeddings_scope_model_checksum_idx;

CREATE UNIQUE INDEX IF NOT EXISTS ob_embeddings_scope_provider_model_kind_checksum_key
  ON ob_embeddings (scope, provider, model, kind, checksum);

CREATE UNIQUE INDEX IF NOT EXISTS ob_embeddings_scope_object_provider_model_kind_key
  ON ob_embeddings (scope, object_id, provider, model, kind)
  WHERE object_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS ob_embeddings_scope_provider_model_kind_idx
  ON ob_embeddings (scope, provider, model, kind);
