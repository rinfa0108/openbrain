-- Preserve every prior canonical object version before replacing the current row.
CREATE TABLE IF NOT EXISTS ob_object_versions (
  object_id             TEXT NOT NULL,
  scope                 TEXT NOT NULL,
  version               BIGINT NOT NULL,
  type                  TEXT NOT NULL,
  status                TEXT NOT NULL,
  spec_version          TEXT NOT NULL,
  tags                  TEXT[] NOT NULL DEFAULT '{}',
  data                  JSONB NOT NULL,
  provenance            JSONB NOT NULL,
  lifecycle_state       TEXT NOT NULL DEFAULT 'accepted',
  expires_at            TIMESTAMPTZ,
  memory_key            TEXT,
  value_hash            TEXT,
  conflict_status       TEXT NOT NULL DEFAULT 'none',
  resolved_by_object_id TEXT,
  resolved_at           TIMESTAMPTZ,
  resolution_note       TEXT,
  captured_at           TIMESTAMPTZ NOT NULL DEFAULT now(),
  PRIMARY KEY (object_id, version)
);

CREATE INDEX IF NOT EXISTS ob_object_versions_scope_idx
  ON ob_object_versions (scope, object_id, version DESC);

CREATE UNIQUE INDEX IF NOT EXISTS ob_objects_scope_id_idx
  ON ob_objects (scope, id);

CREATE OR REPLACE FUNCTION ob_reject_scope_change()
RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
  IF NEW.scope <> OLD.scope THEN
    RAISE EXCEPTION 'object scope is immutable for id %', OLD.id
      USING ERRCODE = 'integrity_constraint_violation';
  END IF;
  RETURN NEW;
END;
$$;

DROP TRIGGER IF EXISTS ob_objects_scope_immutable ON ob_objects;
CREATE TRIGGER ob_objects_scope_immutable
BEFORE UPDATE OF scope ON ob_objects
FOR EACH ROW EXECUTE FUNCTION ob_reject_scope_change();

CREATE OR REPLACE FUNCTION ob_reject_object_version_history_mutation()
RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
  RAISE EXCEPTION 'object version history is append-only'
    USING ERRCODE = 'read_only_sql_transaction';
END;
$$;

DROP TRIGGER IF EXISTS ob_object_versions_no_update ON ob_object_versions;
CREATE TRIGGER ob_object_versions_no_update
BEFORE UPDATE ON ob_object_versions
FOR EACH ROW EXECUTE FUNCTION ob_reject_object_version_history_mutation();

DROP TRIGGER IF EXISTS ob_object_versions_no_delete ON ob_object_versions;
CREATE TRIGGER ob_object_versions_no_delete
BEFORE DELETE ON ob_object_versions
FOR EACH ROW EXECUTE FUNCTION ob_reject_object_version_history_mutation();
