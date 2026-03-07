-- Memory lifecycle metadata (v0.1)

ALTER TABLE ob_objects
  ADD COLUMN IF NOT EXISTS lifecycle_state TEXT NOT NULL DEFAULT 'accepted';

ALTER TABLE ob_objects
  ADD COLUMN IF NOT EXISTS expires_at TIMESTAMPTZ NULL;

ALTER TABLE ob_objects
  ADD COLUMN IF NOT EXISTS memory_key TEXT NULL;

ALTER TABLE ob_objects
  ADD COLUMN IF NOT EXISTS value_hash TEXT NULL;

ALTER TABLE ob_objects
  ADD COLUMN IF NOT EXISTS conflict_status TEXT NOT NULL DEFAULT 'none';

ALTER TABLE ob_objects
  ADD COLUMN IF NOT EXISTS resolved_by_object_id TEXT NULL;

ALTER TABLE ob_objects
  ADD COLUMN IF NOT EXISTS resolved_at TIMESTAMPTZ NULL;

ALTER TABLE ob_objects
  ADD COLUMN IF NOT EXISTS resolution_note TEXT NULL;

CREATE INDEX IF NOT EXISTS ob_objects_scope_lifecycle_idx
  ON ob_objects (scope, lifecycle_state);

CREATE INDEX IF NOT EXISTS ob_objects_scope_expires_idx
  ON ob_objects (scope, expires_at);

CREATE INDEX IF NOT EXISTS ob_objects_scope_memory_key_idx
  ON ob_objects (scope, memory_key);

CREATE INDEX IF NOT EXISTS ob_objects_scope_memory_key_value_hash_idx
  ON ob_objects (scope, memory_key, value_hash);

CREATE INDEX IF NOT EXISTS ob_objects_scope_conflict_status_idx
  ON ob_objects (scope, conflict_status);
