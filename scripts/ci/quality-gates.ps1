$ErrorActionPreference = "Stop"

function Require-Command {
  param([string]$Name)
  if (-not (Get-Command $Name -ErrorAction SilentlyContinue)) {
    Write-Error "Missing required tool: $Name"
    exit 1
  }
}

Write-Host "== Tool Versions =="
Require-Command "rustc"
Require-Command "cargo"

$gitleaksCmd = Get-Command "gitleaks" -ErrorAction SilentlyContinue
if (-not $gitleaksCmd) {
  $wingetRoot = Join-Path $env:LOCALAPPDATA "Microsoft\\WinGet\\Packages"
  if (Test-Path $wingetRoot) {
    $found = Get-ChildItem $wingetRoot -Recurse -Filter gitleaks.exe -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($found) {
      $env:PATH = "$($found.DirectoryName);$env:PATH"
      $gitleaksCmd = $found.FullName
    }
  }
}
if (-not $gitleaksCmd) {
  Write-Error "Missing required tool: gitleaks"
  exit 1
}
& rustc --version
& cargo --version
& cargo deny --version
& gitleaks version

Write-Host "== Rust Gates =="
$env:RUN_OPENAI_LIVE_TESTS = "0"
$env:RUN_ANTHROPIC_LIVE_TESTS = "0"
& cargo fmt --all -- --check
& cargo clippy --all-targets --all-features -- -D warnings
& cargo test --all --all-features

Write-Host "== Dependency Security/Policy =="
& cargo deny check advisories
& cargo deny check licenses bans sources

Write-Host "== Secret Scan =="
& gitleaks detect --source . --config .gitleaks.toml --no-git
