-- Make the object version history append-only at the database level.
--
-- This is a new migration and not an edit to 0006. 0006 has already been applied
-- to databases, and the test harness's migrator records a checksum for every
-- applied file: changing 0006 would leave existing databases without these
-- triggers and make a reused test database refuse to migrate at all.
--
-- Everything here is safe to run again, so it can also be applied by hand to a
-- database that has 0006 but was not created through a migrator.
--
-- The triggers stop the application and ordinary SQL from rewriting recorded
-- history. They are not a defence against a superuser or the table owner, who can
-- disable or drop them; restrict those roles where that matters.

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

-- TRUNCATE does not fire row-level triggers, so it needs its own statement-level
-- one; without it the whole history could be emptied in one command.
DROP TRIGGER IF EXISTS ob_object_versions_no_truncate ON ob_object_versions;
CREATE TRIGGER ob_object_versions_no_truncate
BEFORE TRUNCATE ON ob_object_versions
FOR EACH STATEMENT EXECUTE FUNCTION ob_reject_object_version_history_mutation();
