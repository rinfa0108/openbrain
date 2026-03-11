# Write Idempotency + Receipts

OpenBrain supports replay-safe writes for webhook/event ingestion using `idempotency_key` on `POST /v1/write` and `openbrain.write`.

## Request usage

- Provide `idempotency_key` per logical write batch.
- Key scope is per workspace (`scope`).
- Key length must be `1..256`.

## Replay semantics

1. First write with `(scope, idempotency_key)`:
- Objects/events are written.
- A bounded receipt is persisted.

2. Replay with the same payload:
- No new objects/events are written.
- Prior receipt is returned with `replayed=true`.

3. Reuse with different payload:
- Request is rejected with `OB_INVALID_REQUEST`.
- `error.details.reason_code = OB_IDEMPOTENCY_KEY_REUSE_MISMATCH`.

## Response receipt fields (additive)

- `replayed`: `true` when request was replayed from the ledger.
- `request_id`: deterministic request hash.
- `accepted_count`: total accepted objects.
- `object_ids`: bounded list of written object ids (max `50`).
- `receipt_hash`: deterministic hash of bounded receipt fields.

## Notes

- Receipts never persist auth tokens.
- `accepted_count` can be larger than `object_ids.length` because of bounded receipts.
