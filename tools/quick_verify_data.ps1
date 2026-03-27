param(
  [int]$LimitPerProtocol = 0,
  [double]$MinDecodeOkPct = 65.0,
  [int]$HealthMaxAgeMinutes = 180,
  [int]$MaxAllowedHealthAlerts = 0,
  [switch]$SkipDecodeAudit = $false
)

$ErrorActionPreference = "Stop"

function Info([string]$msg) { Write-Host ("[quick-verify] " + $msg) }

$repoRoot = Split-Path -Parent $PSScriptRoot
Set-Location $repoRoot

$targets = @(
  @{ protocol = "orca"; pool = "Czfq3xZZDmsdGdUyrNLtRhGc47cXcZtLG4crryfu44zE" },
  @{ protocol = "orca"; pool = "HktfL7iwGKT5QHjywQkcDnZXScoh811k7akrMZJkCcEF" },
  @{ protocol = "orca"; pool = "HxA6SKW5qA4o12fjVgTpXdq2YnZ5Zv1s7SB4FFomsyLM" },
  @{ protocol = "raydium"; pool = "3nMFwZXwY1s1M5s8vYAHqd4wGs4iSxXE4LRoUMMYqEgF" },
  @{ protocol = "meteora"; pool = "5rCf1DM8LjKTw4YqhnoLcngyZYeNnQqztScTogYHAS6" },
  @{ protocol = "meteora"; pool = "BGm1tav58oGcsQJehL9WXBFXF7D27vZsKefj4xJKD5Y" },
  @{ protocol = "meteora"; pool = "HTvjzsfX3yU6BUodCjZ5vZkUrAxMDTrBs3CJaq43ashR" }
)

if ($LimitPerProtocol -gt 0) {
  $limited = @()
  foreach ($p in @("orca", "raydium", "meteora")) {
    $limited += $targets | Where-Object { $_.protocol -eq $p } | Select-Object -First $LimitPerProtocol
  }
  $targets = $limited
}

Info ("Snapshot readiness checks: " + $targets.Count + " pools")

$snapshotRows = @()
foreach ($t in $targets) {
  $output = (& cargo run --quiet --bin clmm-lp-cli -- snapshot-readiness --protocol $t.protocol --pool-address $t.pool) | ForEach-Object { "$_" }

  $tier1 = "UNKNOWN"
  $tier2 = "UNKNOWN"
  $tier3 = "UNKNOWN"
  foreach ($line in $output) {
    if ($line -match "1\).+:\s+(READY|NOT READY)") { $tier1 = $Matches[1] }
    if ($line -match "2\).+:\s+(READY|NOT READY)") { $tier2 = $Matches[1] }
    if ($line -match "3\).+:\s+(READY|NOT READY)") { $tier3 = $Matches[1] }
  }

  $snapshotRows += [pscustomobject]@{
    protocol = $t.protocol
    pool = $t.pool
    tier1_lp_share = $tier1
    tier2_snapshot_fee = $tier2
    tier3_position_truth = $tier3
  }
}

$tier12Ready = $snapshotRows | Where-Object { $_.tier1_lp_share -eq "READY" -and $_.tier2_snapshot_fee -eq "READY" }
$snapshotTier12Ok = ($tier12Ready.Count -eq $snapshotRows.Count)

Info ("Running health-check (max_age=${HealthMaxAgeMinutes}m, min_decode_ok=${MinDecodeOkPct}%)")
$healthOut = (& cargo run --quiet --bin clmm-lp-cli -- data-health-check --max-age-minutes $HealthMaxAgeMinutes --min-decode-ok-pct $MinDecodeOkPct) | ForEach-Object { "$_" }
$healthAlerts = -1
foreach ($line in $healthOut) {
  if ($line -match "health summary:\s+alerts=(\d+)") {
    $healthAlerts = [int]$Matches[1]
  }
}
if ($healthAlerts -lt 0) { $healthAlerts = 9999 }
$healthOk = ($healthAlerts -le $MaxAllowedHealthAlerts)

$decodeOkPct = -1.0
if (-not $SkipDecodeAudit) {
  Info "Running decode audit"
  $auditOut = (& cargo run --quiet --bin clmm-lp-cli -- swaps-decode-audit --save-report) | ForEach-Object { "$_" }
  foreach ($line in $auditOut) {
    if ($line -match "decode audit summary:.+ok=\d+\s+\(([\d\.]+)%\)") {
      $decodeOkPct = [double]$Matches[1]
      break
    }
  }
}

$decodeOk = $true
if (-not $SkipDecodeAudit) {
  $decodeOk = ($decodeOkPct -ge $MinDecodeOkPct)
}

$overallGo = $snapshotTier12Ok -and $healthOk -and $decodeOk

$summary = [pscustomobject]@{
  ts_utc = (Get-Date).ToUniversalTime().ToString("o")
  config = @{
    limit_per_protocol = $LimitPerProtocol
    min_decode_ok_pct = $MinDecodeOkPct
    health_max_age_minutes = $HealthMaxAgeMinutes
    max_allowed_health_alerts = $MaxAllowedHealthAlerts
    skip_decode_audit = [bool]$SkipDecodeAudit
  }
  checks = @{
    snapshot_tier12_ok = $snapshotTier12Ok
    health_ok = $healthOk
    health_alerts = $healthAlerts
    decode_ok = $decodeOk
    decode_ok_pct = $decodeOkPct
  }
  snapshot_rows = $snapshotRows
  overall_go = $overallGo
}

$reportsDir = Join-Path $repoRoot "data/reports"
New-Item -ItemType Directory -Path $reportsDir -Force | Out-Null
$stamp = Get-Date -Format "yyyyMMdd_HHmmss"
$reportPath = Join-Path $reportsDir ("quick_verify_" + $stamp + ".json")
$summary | ConvertTo-Json -Depth 8 | Out-File -FilePath $reportPath -Encoding utf8

Write-Host ""
Write-Host "========== QUICK VERIFY =========="
Write-Host ("snapshot_tier12_ok: " + $summary.checks.snapshot_tier12_ok)
Write-Host ("health_ok:          " + $summary.checks.health_ok + " (alerts=" + $summary.checks.health_alerts + ")")
if (-not $SkipDecodeAudit) {
  Write-Host ("decode_ok:          " + $summary.checks.decode_ok + " (ok_pct=" + $summary.checks.decode_ok_pct + "%)")
}
Write-Host ("OVERALL GO:         " + $summary.overall_go)
Write-Host ("report:             " + $reportPath)
Write-Host "=================================="

if (-not $overallGo) {
  exit 2
}

