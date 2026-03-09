$ErrorActionPreference = "Stop"

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
Set-Location $repoRoot

$env:RUN_OPENAI_LIVE_TESTS = "0"
$env:RUN_ANTHROPIC_LIVE_TESTS = "0"

$baseUrl = "http://127.0.0.1:8080"
$dbUrl = "postgres://postgres:postgres@127.0.0.1:5432/openbrain"
$fixturePath = Join-Path $repoRoot "docs/demo/fixtures/transcript_demo.txt"
$stateDir = Join-Path $repoRoot ".openbrain"
$tokenFile = Join-Path $stateDir "demo_tokens.json"
$shadowJson = Join-Path $stateDir "shadow_report.json"
$shadowHtml = Join-Path $stateDir "shadow_report.html"

New-Item -ItemType Directory -Force -Path $stateDir | Out-Null

function Assert-Command {
    param([Parameter(Mandatory = $true)][string]$Name)
    if (-not (Get-Command $Name -ErrorAction SilentlyContinue)) {
        throw "missing required command: $Name"
    }
}

function Assert-DockerReady {
    Assert-Command -Name "docker"
    $null = docker info 2>$null
    if ($LASTEXITCODE -ne 0) {
        throw "docker is not running; start Docker Desktop/Engine and re-run"
    }
}

function Invoke-ObApi {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $false)][string]$Token,
        [Parameter(Mandatory = $true)]$Body
    )
    $uri = "$baseUrl$Path"
    $headers = @{ "Content-Type" = "application/json" }
    if ($Token) {
        $headers["Authorization"] = "Bearer $Token"
    }
    $json = ($Body | ConvertTo-Json -Depth 32 -Compress)
    Invoke-RestMethod -Method Post -Uri $uri -Headers $headers -Body $json
}

function Wait-Ping {
    for ($i = 0; $i -lt 90; $i++) {
        try {
            $resp = Invoke-ObApi -Path "/v1/ping" -Body @{}
            if ($resp.ok -eq $true) {
                return
            }
        }
        catch {
            Start-Sleep -Seconds 1
        }
    }
    throw "openbrain server did not become ready at $baseUrl"
}

function Get-BootstrapOwnerToken {
    $pattern = [regex]'bootstrap owner token \(workspace=(?<workspace>[^)]+)\): (?<token>\S+)'
    for ($i = 0; $i -lt 90; $i++) {
        $logs = docker compose logs openbrain --no-color 2>$null
        $match = $pattern.Match(($logs | Out-String))
        if ($match.Success) {
            return @{
                workspace = $match.Groups["workspace"].Value
                token = $match.Groups["token"].Value
            }
        }
        Start-Sleep -Seconds 1
    }
    throw "could not find bootstrap owner token in container logs"
}

function Invoke-ObCli {
    param([Parameter(Mandatory = $true)][string[]]$Args)
    if (Get-Command openbrain -ErrorAction SilentlyContinue) {
        & openbrain @Args
    }
    else {
        & cargo run -q -p openbrain-server -- @Args
    }
    if ($LASTEXITCODE -ne 0) {
        throw "openbrain CLI command failed: $($Args -join ' ')"
    }
}

function New-DemoText {
    param([Parameter(Mandatory = $true)][string]$Seed)
    $parts = @(
        "Decision context for $Seed in OpenBrain memory.",
        "This block is intentionally verbose for deterministic budget pressure.",
        "Keep governance, lifecycle, and retention constraints visible in the final pack.",
        "Use this memory to answer rollout and migration questions."
    )
    ($parts -join " ") + " " + ($parts -join " ")
}

function Show-PackSummary {
    param(
        [Parameter(Mandatory = $true)]$Resp,
        [Parameter(Mandatory = $true)][int]$Budget,
        [Parameter(Mandatory = $true)][string]$Label
    )
    $pack = $Resp.pack
    Write-Host "`n== Pack ($Label) =="
    Write-Host "budget_requested=$($pack.budget_requested) budget_used=$($pack.budget_used) items_selected=$($pack.items_selected) truncated=$($pack.truncated)"
    Write-Host "pack.items count=$($pack.items.Count)"
    if ($pack.conflict_alerts) {
        Write-Host "conflict_alerts=$($pack.conflict_alerts.Count)"
        $first = $pack.conflict_alerts | Select-Object -First 1
        if ($first) {
            Write-Host "first conflict: key=$($first.memory_key) count=$($first.conflicting_count) status=$($first.conflict_status)"
        }
    }
    else {
        Write-Host "conflict_alerts=0"
    }
    Write-Host "--- first 20 lines of pack.text ---"
    (($pack.text -split "`n") | Select-Object -First 20) | ForEach-Object { Write-Host $_ }
}

Write-Host "== OpenBrain IT12C Memory Pack Demo =="
Write-Host "Preflight: docker"
Assert-DockerReady

Write-Host "Resetting compose environment to a clean state"
docker compose down -v --remove-orphans | Out-Null
docker compose up -d | Out-Null

Write-Host "Waiting for server readiness"
Wait-Ping

$bootstrap = Get-BootstrapOwnerToken
$workspaceId = $bootstrap.workspace
$ownerToken = $bootstrap.token

$writer = Invoke-ObApi -Path "/v1/workspace/token/create" -Token $ownerToken -Body @{
    role = "writer"
    label = "demo-pack-writer"
    display_name = "Demo Pack Writer"
}
if (-not $writer.ok) { throw "writer token create failed" }

$reader = Invoke-ObApi -Path "/v1/workspace/token/create" -Token $ownerToken -Body @{
    role = "reader"
    label = "demo-pack-reader"
    display_name = "Demo Pack Reader"
}
if (-not $reader.ok) { throw "reader token create failed" }

@{
    workspace_id = $workspaceId
    owner_token = $ownerToken
    writer_token = $writer.token
    reader_token = $reader.token
    created_at = (Get-Date).ToUniversalTime().ToString("o")
} | ConvertTo-Json -Depth 8 | Set-Content -Path $tokenFile

Write-Host "workspace=$workspaceId"
Write-Host "tokens saved: $tokenFile"

Write-Host "`nInstalling retention policy and policy deny rule for reader memory packs"
$retention = Invoke-ObApi -Path "/v1/write" -Token $ownerToken -Body @{
    objects = @(
        @{
            type = "policy.retention"
            id = "policy-retention-it12c"
            scope = $workspaceId
            status = "canonical"
            spec_version = "0.1"
            tags = @("demo", "it12c")
            data = @{
                default_ttl_by_kind = @{ scratch = 7; candidate = 30 }
                max_ttl_by_kind = @{ pii = 30 }
                immutable_kinds = @("pii", "credential")
            }
            provenance = @{ actor = "owner" }
            lifecycle_state = "accepted"
        },
        @{
            type = "policy.rule"
            id = "policy-deny-reader-memory-pack-it12c"
            scope = $workspaceId
            status = "canonical"
            spec_version = "0.1"
            tags = @("demo", "it12c", "policy")
            data = @{
                id = "rule-deny-reader-memory-pack-it12c"
                effect = "deny"
                operations = @("memory_pack")
                roles = @("reader")
                reason = "OB_POLICY_DENY_MEMORY_PACK_READER_DEMO"
            }
            provenance = @{ actor = "owner" }
            lifecycle_state = "accepted"
        }
    )
}
if (-not $retention.ok) { throw "failed to install governance objects" }

Write-Host "`nSeeding transcript via shadow mode (write-scratch)"
Invoke-ObCli -Args @(
    "shadow",
    "--database-url", $dbUrl,
    "--workspace", $workspaceId,
    "--token", $writer.token,
    "--mode", "write-scratch",
    "--input", $fixturePath,
    "--format", "text",
    "--limit", "50",
    "--actor", "shadow-demo",
    "--out", $shadowJson,
    "--out-html", $shadowHtml
)

$shadowReport = Get-Content $shadowJson | ConvertFrom-Json
$shadowRefs = @($shadowReport.written_refs)
if ($shadowRefs.Count -lt 3) {
    throw "expected at least 3 shadow refs, got $($shadowRefs.Count)"
}

$shadowRead = Invoke-ObApi -Path "/v1/read" -Token $writer.token -Body @{
    scope = $workspaceId
    refs = $shadowRefs
    include_states = @("scratch", "candidate", "accepted")
}
if (-not $shadowRead.ok) { throw "failed to read shadow refs" }

$updates = @()
$i = 0
foreach ($obj in $shadowRead.objects) {
    $state = "scratch"
    $status = "active"
    if ($i -lt 2) {
        $state = "accepted"
        $status = "canonical"
    }
    elseif ($i -eq 2) {
        $state = "candidate"
        $status = "candidate"
    }
    $updates += @{
        type = $obj.type
        id = $obj.ref
        scope = $workspaceId
        status = $status
        spec_version = "0.1"
        tags = @("demo", "shadow", "it12c")
        data = $obj.data
        provenance = @{ actor = "writer"; source = "shadow-promote" }
        lifecycle_state = $state
        memory_key = $obj.memory_key
    }
    $i++
}

for ($n = 1; $n -le 8; $n++) {
    $id = ("demo-pack-context-{0:D2}" -f $n)
    $updates += @{
        type = "context"
        id = $id
        scope = $workspaceId
        status = "canonical"
        spec_version = "0.1"
        tags = @("demo", "it12c", "pack")
        data = @{ text = (New-DemoText -Seed $id) }
        provenance = @{ actor = "writer" }
        lifecycle_state = "accepted"
        memory_key = "context:$id"
    }
}

$updates += @(
    @{
        type = "note"
        id = "demo-hidden-scratch"
        scope = $workspaceId
        status = "draft"
        spec_version = "0.1"
        tags = @("demo", "it12c")
        data = @{ text = "scratch object should be excluded by default pack retrieval" }
        provenance = @{ actor = "writer" }
        lifecycle_state = "scratch"
        memory_key = "note:hidden-scratch"
    },
    @{
        type = "note"
        id = "demo-hidden-candidate"
        scope = $workspaceId
        status = "candidate"
        spec_version = "0.1"
        tags = @("demo", "it12c")
        data = @{ text = "candidate object should be excluded by default pack retrieval" }
        provenance = @{ actor = "writer" }
        lifecycle_state = "candidate"
        memory_key = "note:hidden-candidate"
    },
    @{
        type = "decision"
        id = "demo-conflict-a"
        scope = $workspaceId
        status = "canonical"
        spec_version = "0.1"
        tags = @("demo", "it12c", "conflict")
        data = @{ choice = "postgres"; rationale = "operational simplicity" }
        provenance = @{ actor = "writer" }
        lifecycle_state = "accepted"
        memory_key = "decision:db"
        conflict_status = "unresolved"
    },
    @{
        type = "decision"
        id = "demo-conflict-b"
        scope = $workspaceId
        status = "canonical"
        spec_version = "0.1"
        tags = @("demo", "it12c", "conflict")
        data = @{ choice = "sqlite"; rationale = "single file dev speed" }
        provenance = @{ actor = "writer" }
        lifecycle_state = "accepted"
        memory_key = "decision:db"
        conflict_status = "unresolved"
    }
)

$seedWrite = Invoke-ObApi -Path "/v1/write" -Token $writer.token -Body @{ objects = $updates }
if (-not $seedWrite.ok) { throw "seed write failed" }
Write-Host "seeded deterministic objects: $($updates.Count)"

Write-Host "`nGenerating initial embedding in legacy space (fake/fake-v1)"
$embedLegacy = Invoke-ObApi -Path "/v1/embed/generate" -Token $writer.token -Body @{
    scope = $workspaceId
    target = @{ ref = "demo-pack-context-01" }
    model = "fake-v1"
}
if (-not $embedLegacy.ok) { throw "legacy embed.generate failed" }

$budgets = @(400, 1200, 1600)
foreach ($budget in $budgets) {
    $pack = Invoke-ObApi -Path "/v1/memory/pack" -Token $writer.token -Body @{
        scope = $workspaceId
        task_hint = "Prepare response context for database and rollout decisions"
        query = "database decision rollout context"
        semantic = $false
        budget_tokens = $budget
        top_k = 30
        max_per_key = 1
        include_conflicts = $true
    }
    if (-not $pack.ok) { throw "memory pack failed for budget $budget" }
    Show-PackSummary -Resp $pack -Budget $budget -Label "$budget tokens"
}

Write-Host "`n== Conflict Detail Toggle =="
$packDefaultConflict = Invoke-ObApi -Path "/v1/memory/pack" -Token $writer.token -Body @{
    scope = $workspaceId
    task_hint = "Compare unresolved conflict behavior"
    query = "decision db conflict"
    semantic = $false
    budget_tokens = 1200
    structured_filter = 'memory_key == "decision:db"'
    include_conflicts = $true
    include_conflicts_detail = $false
}
$packDetailConflict = Invoke-ObApi -Path "/v1/memory/pack" -Token $writer.token -Body @{
    scope = $workspaceId
    task_hint = "Compare unresolved conflict behavior"
    query = "decision db conflict"
    semantic = $false
    budget_tokens = 1200
    structured_filter = 'memory_key == "decision:db"'
    include_conflicts = $true
    include_conflicts_detail = $true
}
if (-not $packDefaultConflict.ok -or -not $packDetailConflict.ok) {
    throw "conflict comparison memory pack call failed"
}
$defaultAlert = $packDefaultConflict.pack.conflict_alerts | Select-Object -First 1
$detailAlert = $packDetailConflict.pack.conflict_alerts | Select-Object -First 1
Write-Host "default detail: conflicting_object_ids populated=$([bool]$defaultAlert.conflicting_object_ids)"
Write-Host "include_conflicts_detail=true: conflicting_object_ids count=$(@($detailAlert.conflicting_object_ids).Count)"

Write-Host "`n== Policy Deny Explainability =="
$denied = Invoke-ObApi -Path "/v1/memory/pack" -Token $reader.token -Body @{
    scope = $workspaceId
    task_hint = "reader should be denied by policy"
    query = "database decision"
    semantic = $false
    budget_tokens = 400
}
if ($denied.ok -ne $false) {
    throw "expected reader memory pack to be denied by policy"
}
$reason = $denied.error.details.reason_code
$rule = $denied.error.details.policy_rule_id
Write-Host "OB_FORBIDDEN reason_code=$reason policy_rule_id=$rule"

Write-Host "`n== Embedding Migration (Coverage + Reembed) =="
Write-Host "Coverage before target space fake/fake-v2:"
Invoke-ObCli -Args @(
    "embed",
    "--database-url", $dbUrl,
    "--token", $writer.token,
    "coverage",
    "--workspace", $workspaceId,
    "--provider", "fake",
    "--model", "fake-v2",
    "--kind", "semantic",
    "--state", "accepted",
    "--missing-sample", "5"
)

Write-Host "`nReembed dry-run:"
Invoke-ObCli -Args @(
    "embed",
    "--database-url", $dbUrl,
    "--token", $writer.token,
    "reembed",
    "--workspace", $workspaceId,
    "--to-provider", "fake",
    "--to-model", "fake-v2",
    "--to-kind", "semantic",
    "--state", "accepted",
    "--limit", "20",
    "--dry-run"
)

Write-Host "`nReembed execute:"
Invoke-ObCli -Args @(
    "embed",
    "--database-url", $dbUrl,
    "--token", $writer.token,
    "reembed",
    "--workspace", $workspaceId,
    "--to-provider", "fake",
    "--to-model", "fake-v2",
    "--to-kind", "semantic",
    "--state", "accepted",
    "--limit", "20",
    "--max-objects", "20",
    "--max-bytes", "262144",
    "--actor", "demo-reembed"
)

Write-Host "`nCoverage after target space fake/fake-v2:"
Invoke-ObCli -Args @(
    "embed",
    "--database-url", $dbUrl,
    "--token", $writer.token,
    "coverage",
    "--workspace", $workspaceId,
    "--provider", "fake",
    "--model", "fake-v2",
    "--kind", "semantic",
    "--state", "accepted",
    "--missing-sample", "5"
)

$packSemantic = Invoke-ObApi -Path "/v1/memory/pack" -Token $writer.token -Body @{
    scope = $workspaceId
    task_hint = "semantic pack in migrated embedding space"
    query = "database decision"
    semantic = $true
    embedding_provider = "fake"
    embedding_model = "fake-v2"
    embedding_kind = "semantic"
    budget_tokens = 800
    top_k = 20
}
if (-not $packSemantic.ok) { throw "semantic pack after migration failed" }
Write-Host "`nSemantic pack (fake-v2) items_selected=$($packSemantic.pack.items_selected) budget_used=$($packSemantic.pack.budget_used)"

Write-Host "`nDemo complete."
Write-Host "- Workspace seeded with lifecycle states and conflicts"
Write-Host "- Memory pack shown at 400 / 1200 / 1600 budgets"
Write-Host "- Policy deny explainability confirmed (reason_code + policy_rule_id)"
Write-Host "- Coverage before/after and reembed workflow executed"
Write-Host "- Tokens/report files:"
Write-Host "  $tokenFile"
Write-Host "  $shadowJson"
Write-Host "  $shadowHtml"
Write-Host "- Next step: open $baseUrl/viewer and inspect workspace/audit/object panels."
