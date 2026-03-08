$ErrorActionPreference = "Stop"

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
Set-Location $repoRoot

$env:RUN_OPENAI_LIVE_TESTS = "0"
$env:RUN_ANTHROPIC_LIVE_TESTS = "0"

$tokenDir = Join-Path $repoRoot ".openbrain"
$tokenFile = Join-Path $tokenDir "demo_tokens.json"
New-Item -ItemType Directory -Path $tokenDir -Force | Out-Null

function Invoke-OpenBrain {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $false)][string]$Token,
        [Parameter(Mandatory = $true)]$Body
    )

    $uri = "http://127.0.0.1:8080$Path"
    $headers = @{ "Content-Type" = "application/json" }
    if ($Token) {
        $headers["Authorization"] = "Bearer $Token"
    }

    $json = ($Body | ConvertTo-Json -Depth 16 -Compress)
    Invoke-RestMethod -Method Post -Uri $uri -Headers $headers -Body $json
}

function Wait-Ping {
    for ($i = 0; $i -lt 60; $i++) {
        try {
            $resp = Invoke-OpenBrain -Path "/v1/ping" -Body @{}
            if ($resp.ok -eq $true) {
                return
            }
        }
        catch {
            Start-Sleep -Seconds 1
        }
    }
    throw "openbrain server did not become ready on http://127.0.0.1:8080"
}

function Get-BootstrapOwnerToken {
    $pattern = [regex]'bootstrap owner token \(workspace=(?<workspace>[^)]+)\): (?<token>\S+)'
    for ($i = 0; $i -lt 60; $i++) {
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
    throw "could not find bootstrap owner token in openbrain logs; run on a fresh compose volume or inspect 'docker compose logs openbrain'"
}

Write-Host "== OpenBrain IT11A Demo =="
Write-Host "Ensuring services are up (docker compose up -d)..."
docker compose up -d | Out-Null

Write-Host "Waiting for /v1/ping..."
Wait-Ping

$bootstrap = Get-BootstrapOwnerToken
$ownerToken = $bootstrap.token
$workspaceId = $bootstrap.workspace

$writer = Invoke-OpenBrain -Path "/v1/workspace/token/create" -Token $ownerToken -Body @{
    role = "writer"
    label = "demo-writer"
    display_name = "Demo Writer"
}
if (-not $writer.ok) { throw "failed to create writer token: $($writer.error | ConvertTo-Json -Compress)" }

$reader = Invoke-OpenBrain -Path "/v1/workspace/token/create" -Token $ownerToken -Body @{
    role = "reader"
    label = "demo-reader"
    display_name = "Demo Reader"
}
if (-not $reader.ok) { throw "failed to create reader token: $($reader.error | ConvertTo-Json -Compress)" }

$tokens = @{
    workspace_id = $workspaceId
    owner_token = $ownerToken
    writer_token = $writer.token
    reader_token = $reader.token
    created_at = (Get-Date).ToString("o")
}
$tokens | ConvertTo-Json -Depth 4 | Set-Content -Path $tokenFile

Write-Host "workspace: $workspaceId"
Write-Host "owner token (printed once): $ownerToken"
Write-Host "writer token (printed once): $($writer.token)"
Write-Host "reader token (printed once): $($reader.token)"
Write-Host "tokens saved: $tokenFile"

Write-Host "\n== Workspace Info =="
$workspaceInfo = Invoke-OpenBrain -Path "/v1/workspace/info" -Token $ownerToken -Body @{}
$workspaceInfo | ConvertTo-Json -Depth 8

$retentionId = "policy-retention-demo"
$retentionWrite = Invoke-OpenBrain -Path "/v1/write" -Token $ownerToken -Body @{
    objects = @(
        @{
            type = "policy.retention"
            id = $retentionId
            scope = $workspaceId
            status = "canonical"
            spec_version = "0.1"
            tags = @("demo")
            data = @{
                default_ttl_by_kind = @{ scratch = 7; candidate = 30 }
                max_ttl_by_kind = @{ pii = 30 }
                immutable_kinds = @("pii", "credential")
            }
            provenance = @{ actor = "owner" }
            lifecycle_state = "accepted"
        }
    )
}
if (-not $retentionWrite.ok) { throw "failed to install policy.retention: $($retentionWrite.error | ConvertTo-Json -Compress)" }
Write-Host "\nInstalled policy.retention object: $retentionId"

$lifecycleWrite = Invoke-OpenBrain -Path "/v1/write" -Token $writer.token -Body @{
    objects = @(
        @{
            type = "note"
            id = "demo-scratch"
            scope = $workspaceId
            status = "draft"
            spec_version = "0.1"
            tags = @("demo", "lifecycle")
            data = @{ text = "scratch memory" }
            provenance = @{ actor = "writer" }
            lifecycle_state = "scratch"
        },
        @{
            type = "note"
            id = "demo-candidate"
            scope = $workspaceId
            status = "candidate"
            spec_version = "0.1"
            tags = @("demo", "lifecycle")
            data = @{ text = "candidate memory" }
            provenance = @{ actor = "writer" }
            lifecycle_state = "candidate"
        },
        @{
            type = "note"
            id = "demo-accepted"
            scope = $workspaceId
            status = "canonical"
            spec_version = "0.1"
            tags = @("demo", "lifecycle")
            data = @{ text = "accepted memory" }
            provenance = @{ actor = "writer" }
            lifecycle_state = "accepted"
        }
    )
}
if (-not $lifecycleWrite.ok) { throw "failed lifecycle write: $($lifecycleWrite.error | ConvertTo-Json -Compress)" }

Write-Host "\n== Lifecycle Retrieval =="
$defaultRead = Invoke-OpenBrain -Path "/v1/read" -Token $writer.token -Body @{
    scope = $workspaceId
    refs = @("demo-scratch", "demo-candidate", "demo-accepted")
}
if ($defaultRead.ok) {
    Write-Host "default read count (accepted + not expired): $($defaultRead.objects.Count)"
}

$overrideRead = Invoke-OpenBrain -Path "/v1/read" -Token $writer.token -Body @{
    scope = $workspaceId
    refs = @("demo-scratch", "demo-candidate", "demo-accepted")
    include_states = @("scratch", "candidate", "accepted")
    include_expired = $true
}
if ($overrideRead.ok) {
    Write-Host "override read count (scratch/candidate/accepted): $($overrideRead.objects.Count)"
}

$embed = Invoke-OpenBrain -Path "/v1/embed/generate" -Token $writer.token -Body @{
    scope = $workspaceId
    target = @{ ref = "demo-accepted" }
    model = "fake-v1"
}
if (-not $embed.ok) { throw "embed.generate failed: $($embed.error | ConvertTo-Json -Compress)" }

$semantic = Invoke-OpenBrain -Path "/v1/search/semantic" -Token $writer.token -Body @{
    scope = $workspaceId
    query = "accepted memory"
    top_k = 5
    embedding_provider = "fake"
    embedding_model = "fake-v1"
    embedding_kind = "semantic"
}
if (-not $semantic.ok) { throw "semantic search failed: $($semantic.error | ConvertTo-Json -Compress)" }
Write-Host "\nsemantic matches: $($semantic.matches.Count) (provider=fake, model=fake-v1, kind=semantic)"

$conflictWrite = Invoke-OpenBrain -Path "/v1/write" -Token $writer.token -Body @{
    objects = @(
        @{
            type = "decision"
            id = "demo-conflict-a"
            scope = $workspaceId
            status = "canonical"
            spec_version = "0.1"
            tags = @("demo", "conflict")
            data = @{ choice = "postgres" }
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
            tags = @("demo", "conflict")
            data = @{ choice = "sqlite" }
            provenance = @{ actor = "writer" }
            lifecycle_state = "accepted"
            memory_key = "decision:db"
            conflict_status = "unresolved"
        }
    )
}
if (-not $conflictWrite.ok) { throw "conflict setup write failed: $($conflictWrite.error | ConvertTo-Json -Compress)" }

$conflictSearch = Invoke-OpenBrain -Path "/v1/search/structured" -Token $writer.token -Body @{
    scope = $workspaceId
    where_expr = 'memory_key == "decision:db"'
    include_conflicts = $true
    limit = 20
    offset = 0
}
if (-not $conflictSearch.ok) { throw "conflict search failed: $($conflictSearch.error | ConvertTo-Json -Compress)" }
$firstConflict = $conflictSearch.results | Select-Object -First 1
Write-Host "\nconflict before resolution: conflict=$($firstConflict.conflict) count=$($firstConflict.conflict_count)"

$now = (Get-Date).ToUniversalTime().ToString("o")
$resolveWrite = Invoke-OpenBrain -Path "/v1/write" -Token $writer.token -Body @{
    objects = @(
        @{
            type = "decision"
            id = "demo-conflict-a"
            scope = $workspaceId
            status = "canonical"
            spec_version = "0.1"
            tags = @("demo", "conflict")
            data = @{ choice = "postgres" }
            provenance = @{ actor = "writer" }
            lifecycle_state = "accepted"
            memory_key = "decision:db"
            conflict_status = "resolved"
            resolved_by_object_id = "demo-conflict-a"
            resolved_at = $now
            resolution_note = "demo winner"
        },
        @{
            type = "decision"
            id = "demo-conflict-b"
            scope = $workspaceId
            status = "deprecated"
            spec_version = "0.1"
            tags = @("demo", "conflict")
            data = @{ choice = "sqlite" }
            provenance = @{ actor = "writer" }
            lifecycle_state = "deprecated"
            memory_key = "decision:db"
            conflict_status = "resolved"
            resolved_by_object_id = "demo-conflict-a"
            resolved_at = $now
            resolution_note = "deprecated after resolution"
        }
    )
}
if (-not $resolveWrite.ok) { throw "conflict resolution write failed: $($resolveWrite.error | ConvertTo-Json -Compress)" }

$conflictSearchAfter = Invoke-OpenBrain -Path "/v1/search/structured" -Token $writer.token -Body @{
    scope = $workspaceId
    where_expr = 'memory_key == "decision:db"'
    include_conflicts = $true
    include_states = @("accepted", "deprecated")
    limit = 20
    offset = 0
}
$winner = $conflictSearchAfter.results | Where-Object { $_.ref -eq "demo-conflict-a" } | Select-Object -First 1
if ($winner) {
    Write-Host "conflict after resolution (winner): status=$($winner.conflict_status) resolved_by=$($winner.resolved_by_object_id)"
}

$denyPolicy = Invoke-OpenBrain -Path "/v1/write" -Token $ownerToken -Body @{
    objects = @(
        @{
            type = "policy.rule"
            id = "demo-policy-deny-reader-audit"
            scope = $workspaceId
            status = "canonical"
            spec_version = "0.1"
            tags = @("demo", "policy")
            data = @{
                id = "demo-deny-reader-audit"
                effect = "deny"
                operations = @("audit_object_timeline")
                roles = @("reader")
                reason = "OB_POLICY_DENY_AUDIT_READ_DEMO"
            }
            provenance = @{ actor = "owner" }
            lifecycle_state = "accepted"
        }
    )
}
if (-not $denyPolicy.ok) { throw "failed to install deny policy: $($denyPolicy.error | ConvertTo-Json -Compress)" }

$readerDenied = Invoke-OpenBrain -Path "/v1/audit/object_timeline" -Token $reader.token -Body @{
    scope = $workspaceId
    object_id = "demo-accepted"
    limit = 20
}
if ($readerDenied.ok -eq $false) {
    $reason = $readerDenied.error.details.reason_code
    $rule = $readerDenied.error.details.policy_rule_id
    Write-Host "\nDENIED: $reason (rule: $rule)"
}

Write-Host "\n== Audit Timelines =="
$auditObject = Invoke-OpenBrain -Path "/v1/audit/object_timeline" -Token $ownerToken -Body @{
    scope = $workspaceId
    object_id = "demo-conflict-a"
    limit = 20
}
if ($auditObject.ok) {
    Write-Host "object timeline events: $($auditObject.events.Count)"
}

$auditActor = Invoke-OpenBrain -Path "/v1/audit/actor_activity" -Token $ownerToken -Body @{
    scope = $workspaceId
    actor_identity_id = "writer"
    limit = 20
}
if ($auditActor.ok) {
    Write-Host "actor timeline events (writer): $($auditActor.events.Count)"
}

Write-Host "\nDemo complete."
Write-Host "- Workspace info shown"
Write-Host "- Tokens created and saved"
Write-Host "- policy.retention installed"
Write-Host "- Lifecycle defaults + override shown"
Write-Host "- Semantic search (fake embeddings) shown"
Write-Host "- Conflict detection + resolution shown"
Write-Host "- Deny explainability shown"
Write-Host "- Audit timelines shown"