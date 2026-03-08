# Re-embed Coverage Quickstart

## What this solves

When you switch embedding spaces (provider/model/kind), semantic coverage can drop because older objects are only embedded in the previous space.  
`openbrain embed coverage` and `openbrain embed reembed` let operators measure and fill that gap in bounded, resumable batches.

## Coverage report

```powershell
openbrain embed coverage `
  --workspace default `
  --provider fake `
  --model fake-v1 `
  --kind semantic `
  --state accepted `
  --missing-sample 10 `
  --database-url $env:DATABASE_URL
```

Output includes:
- `total_eligible`
- `with_embeddings`
- `missing`
- `percent_coverage`
- capped sample of `missing_refs`

## Dry-run re-embed

```powershell
openbrain embed --token $env:OPENBRAIN_TOKEN reembed `
  --workspace default `
  --to-provider fake `
  --to-model fake-v1 `
  --to-kind semantic `
  --state accepted `
  --limit 100 `
  --dry-run `
  --database-url $env:DATABASE_URL
```

`--dry-run` shows what would be processed without writing embeddings.

## Execute re-embed (bounded + resumable)

```powershell
openbrain embed --token $env:OPENBRAIN_TOKEN reembed `
  --workspace default `
  --to-provider fake `
  --to-model fake-v1 `
  --to-kind semantic `
  --limit 100 `
  --max-objects 100 `
  --max-bytes 262144 `
  --database-url $env:DATABASE_URL
```

Use the returned `next_cursor` to resume:

```powershell
openbrain embed --token $env:OPENBRAIN_TOKEN reembed `
  --workspace default `
  --to-provider fake `
  --to-model fake-v1 `
  --cursor <next_cursor> `
  --limit 100 `
  --database-url $env:DATABASE_URL
```

## Safe provider switching workflow

1. Choose the target embedding space (`provider/model/kind`).
2. Run `coverage` to baseline.
3. Run `reembed` in dry-run mode.
4. Run bounded real batches until coverage reaches target.
5. Re-run semantic search with the target embedding selector.

For deterministic local runs, use `--to-provider fake` and keep live providers disabled unless explicitly needed.
