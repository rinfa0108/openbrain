# Releasing OpenBrain

This runbook covers release hygiene for tagged versions (including v1.0.0).

## Pre-release checks
1. Confirm branch is up to date and release-ready.
2. Run local gates:
   - `pwsh scripts/ci/quality-gates.ps1`
3. Confirm versions are aligned:
   - Rust crates in `crates/*/Cargo.toml`
   - TS SDK in `sdk/typescript/openbrain-sdk/package.json`
   - Python SDK in `sdk/python/openbrain_sdk/pyproject.toml`
4. Confirm docs are accurate for onboarding and compatibility.

## Tagging
```bash
git checkout main
git pull --ff-only
git tag v1.0.0
git push origin v1.0.0
```

## Build binaries (optional)
If distributing CLI binaries, build from the server crate:
```bash
cargo build -p openbrain-server --release
```
Binary path:
- `target/release/openbrain`

## SDK publish steps (optional, not part of this PR)
### TypeScript
```bash
cd sdk/typescript/openbrain-sdk
npm pack
# publish flow is org-controlled
```

### Python
```bash
cd sdk/python/openbrain_sdk
python -m build
# publish flow is org-controlled
```

## Release notes
- Update `CHANGELOG.md` for the release.
- Include gate output and compatibility notes in release PR.
- Publish GitHub release with tag + changelog summary.