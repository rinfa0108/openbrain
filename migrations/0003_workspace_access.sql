-- Workspace + access control (IT9A)
CREATE TABLE IF NOT EXISTS ob_workspaces (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS ob_identities (
    id TEXT PRIMARY KEY,
    display_name TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS ob_tokens (
    token_hash TEXT PRIMARY KEY,
    identity_id TEXT NOT NULL REFERENCES ob_identities(id) ON DELETE CASCADE,
    workspace_id TEXT NOT NULL REFERENCES ob_workspaces(id) ON DELETE CASCADE,
    role TEXT NOT NULL CHECK (role IN ('owner','writer','reader')),
    label TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    revoked_at TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS idx_ob_tokens_workspace ON ob_tokens (workspace_id);
CREATE INDEX IF NOT EXISTS idx_ob_tokens_identity ON ob_tokens (identity_id);
