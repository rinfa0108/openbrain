#!/usr/bin/env bash
set -euo pipefail

require_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "Missing required tool: $1" >&2
    exit 1
  fi
}

echo "== Tool Versions =="
require_cmd rustc
require_cmd cargo
require_cmd gitleaks
rustc --version
cargo --version
cargo deny --version
gitleaks version

echo "== Rust Gates =="
export RUN_OPENAI_LIVE_TESTS=0
export RUN_ANTHROPIC_LIVE_TESTS=0
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all --all-features

echo "== Dependency Security/Policy =="
cargo deny check advisories
cargo deny check licenses bans sources

echo "== Secret Scan =="
gitleaks detect --source . --config .gitleaks.toml --no-git
