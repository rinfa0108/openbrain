-- Replay-safe write idempotency ledger

CREATE TABLE IF NOT EXISTS ob_idempotency (
  scope TEXT NOT NULL,
  idempotency_key TEXT NOT NULL,
  request_hash TEXT NOT NULL,
  receipt_hash TEXT NOT NULL,
  accepted_count INTEGER NOT NULL,
  object_ids TEXT[] NOT NULL DEFAULT '{}',
  results_json JSONB NOT NULL DEFAULT '[]'::jsonb,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  PRIMARY KEY (scope, idempotency_key)
);

CREATE INDEX IF NOT EXISTS ob_idempotency_scope_created_idx
  ON ob_idempotency (scope, created_at DESC);
