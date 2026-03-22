$ErrorActionPreference = "Stop"

function Require-Command {
  param([string]$Name)
  if (-not (Get-Command $Name -ErrorAction SilentlyContinue)) {
    Write-Error "Missing required tool: $Name"
    exit 1
  }
}

function Invoke-RequiredCommand {
  param(
    [string]$Command,
    [string[]]$Arguments
  )
  & $Command @Arguments
  $exitCode = $LASTEXITCODE
  if ($exitCode -ne 0) {
    Write-Error "Command failed: $Command $($Arguments -join ' ') (exit $exitCode)"
    exit $exitCode
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
$gitleaksExe = if ($gitleaksCmd -is [System.Management.Automation.CommandInfo]) { $gitleaksCmd.Source } else { [string]$gitleaksCmd }
Invoke-RequiredCommand -Command "rustc" -Arguments @("--version")
Invoke-RequiredCommand -Command "cargo" -Arguments @("--version")
Invoke-RequiredCommand -Command "cargo" -Arguments @("deny", "--version")
Invoke-RequiredCommand -Command $gitleaksExe -Arguments @("version")

Write-Host "== Rust Gates =="
$env:RUN_OPENAI_LIVE_TESTS = "0"
$env:RUN_ANTHROPIC_LIVE_TESTS = "0"
Invoke-RequiredCommand -Command "cargo" -Arguments @("fmt", "--all", "--", "--check")
Invoke-RequiredCommand -Command "cargo" -Arguments @("clippy", "--all-targets", "--all-features", "--", "-D", "warnings")
Invoke-RequiredCommand -Command "cargo" -Arguments @("test", "--all", "--all-features")

Write-Host "== Dependency Security/Policy =="
Invoke-RequiredCommand -Command "cargo" -Arguments @("deny", "check", "advisories")
Invoke-RequiredCommand -Command "cargo" -Arguments @("deny", "check", "licenses", "bans", "sources")

Write-Host "== Secret Scan =="
Invoke-RequiredCommand -Command $gitleaksExe -Arguments @("detect", "--source", ".", "--config", ".gitleaks.toml", "--no-git")
