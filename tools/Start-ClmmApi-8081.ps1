#Starts clmm-lp-api on :8081 without touching :8080 (Jenkins).

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
Set-Location $RepoRoot
$WorkCacheRoot = Join-Path $RepoRoot ".cache"
$NpmCacheDir = Join-Path $WorkCacheRoot "npm-cache"
$TempDir = Join-Path $WorkCacheRoot "tmp"
New-Item -ItemType Directory -Force -Path $NpmCacheDir | Out-Null
New-Item -ItemType Directory -Force -Path $TempDir | Out-Null
$env:NPM_CONFIG_CACHE = $NpmCacheDir
$env:TEMP = $TempDir
$env:TMP = $TempDir

# Ensure API isn't already running from previous session
try { & (Join-Path $RepoRoot "tools\\Stop-ClmmApi.ps1") | Out-Null } catch {}

$env:API_PORT = "8081"
$env:CLMM_REPO_ROOT = $RepoRoot
# Local interactive dashboard expects real tx execution by default.
# Keep explicit user override if DRY_RUN was already set.
if (-not $env:DRY_RUN -or $env:DRY_RUN.Trim().Length -eq 0) {
  $env:DRY_RUN = "false"
}

# Build/run isolation: use a dedicated Cargo target dir so rebuilds don't fail with
# "failed to remove ...\\target\\debug\\clmm-lp-api.exe" when another instance is running.
if (-not $env:CLMM_API_TARGET_DIR -or $env:CLMM_API_TARGET_DIR.Trim().Length -eq 0) {
  $env:CLMM_API_TARGET_DIR = "target-dev-api"
}

# Ensure signer + stranded-watchdog env vars are present.
# We prefer existing process env; otherwise load from `.env` best-effort.
$signerVars = @(
  "KEYPAIR_PATH",
  "SOLANA_KEYPAIR_PATH",
  "WALLET_KEYPAIR_PATH",
  "SOLANA_KEYPAIR",
  "WALLET_KEYPAIR_BASE58"
)
$watchdogEnvVars = @(
  "CLMM_STRANDED_RECONCILE_INTERVAL_SECS",
  "CLMM_IL_LEDGER_PATH",
  "CLMM_PENDING_OPEN_RECOVERY_PATH"
)
# Chain-history / stream DB / wallet_gl migrations require Postgres; without this the API serves 503 on DB routes.
$dbEnvVars = @(
  "DATABASE_URL",
  "DATABASE_POOL_SIZE"
)
$envVarsToHydrateFromDotenv = $signerVars + $watchdogEnvVars + $dbEnvVars
$envFile = Join-Path $RepoRoot ".env"
if (Test-Path -LiteralPath $envFile) {
  $envLines = Get-Content -LiteralPath $envFile -ErrorAction SilentlyContinue
  foreach ($name in $envVarsToHydrateFromDotenv) {
    $existing = Get-Item -Path "Env:$name" -ErrorAction SilentlyContinue
    if ($existing -and $existing.Value -and $existing.Value.Trim().Length -gt 0) { continue }
    $line = $envLines | Where-Object { $_ -match "^\s*$name\s*=" } | Select-Object -First 1
    if (-not $line) { continue }
    $value = ($line -replace "^\s*$name\s*=\s*", "").Trim()
    if ($value.Length -ge 2 -and (($value.StartsWith('"') -and $value.EndsWith('"')) -or ($value.StartsWith("'") -and $value.EndsWith("'")))) {
      $value = $value.Substring(1, $value.Length - 2)
    }
    if ($value.Length -gt 0) { Set-Item -Path "Env:$name" -Value $value }
  }
}

# If the script runner is configured to use a non-default port, align API env override.
if ($env:CLMM_SCRIPT_RUNNER_PORT -and $env:CLMM_SCRIPT_RUNNER_PORT -match '^\d+$') {
  $env:SCRIPT_RUNNER_URL = "http://127.0.0.1:$($env:CLMM_SCRIPT_RUNNER_PORT)"
}

# If caller didn't set an explicit CLI path, prefer fresh debug CLI when available.
$debugCli = Join-Path $RepoRoot "target\debug\clmm-lp-cli.exe"
if ((-not $env:CLMM_LP_CLI_PATH -or $env:CLMM_LP_CLI_PATH.Trim().Length -eq 0) -and (Test-Path -LiteralPath $debugCli)) {
  $env:CLMM_LP_CLI_PATH = $debugCli
}

Write-Host "[Start-ClmmApi-8081] Starting API on :8081 (does not touch :8080)..." -ForegroundColor Cyan
Write-Host "[Start-ClmmApi-8081] Cargo target dir: $env:CLMM_API_TARGET_DIR" -ForegroundColor DarkGray
Write-Host "[Start-ClmmApi-8081] DRY_RUN=$env:DRY_RUN" -ForegroundColor DarkGray
Write-Host "[Start-ClmmApi-8081] CLMM_LP_CLI_PATH=$env:CLMM_LP_CLI_PATH" -ForegroundColor DarkGray
Write-Host "[Start-ClmmApi-8081] TEMP/TMP: $env:TEMP" -ForegroundColor DarkGray
$signerSummary = $signerVars | ForEach-Object {
  $v = Get-Item -Path "Env:$_" -ErrorAction SilentlyContinue
  if ($v -and $v.Value -and $v.Value.Trim().Length -gt 0) { "$_=set" } else { "$_=unset" }
}
Write-Host ("[Start-ClmmApi-8081] signer env: " + ($signerSummary -join ", ")) -ForegroundColor DarkGray
$watchdogSummary = $watchdogEnvVars | ForEach-Object {
  $v = Get-Item -Path "Env:$_" -ErrorAction SilentlyContinue
  if ($v -and $v.Value -and $v.Value.Trim().Length -gt 0) { "$_=set" } else { "$_=unset" }
}
Write-Host ("[Start-ClmmApi-8081] watchdog/IL env: " + ($watchdogSummary -join ", ")) -ForegroundColor DarkGray
$dbSummary = $dbEnvVars | ForEach-Object {
  $v = Get-Item -Path "Env:$_" -ErrorAction SilentlyContinue
  if ($v -and $v.Value -and $v.Value.Trim().Length -gt 0) { "$_=set" } else { "$_=unset" }
}
Write-Host ("[Start-ClmmApi-8081] database env: " + ($dbSummary -join ", ")) -ForegroundColor DarkGray

$logDir = Join-Path $RepoRoot "tools\\logs"
New-Item -ItemType Directory -Force -Path $logDir | Out-Null
$ts = Get-Date -Format "yyyyMMdd_HHmmss"
$logPath = Join-Path $logDir ("clmm-lp-api_8081_{0}.log" -f $ts)

# Build here (blocking) so Start-Dashboard-Safe health-wait is not racing a hidden `cargo run` compile.
# After Rust changes, first rebuild of protocols/execution/api can exceed 180s; runtime startup stays ~30–40s.
Write-Host "[Start-ClmmApi-8081] Building clmm-lp-api (target-dev-api)..." -ForegroundColor Cyan
$buildSw = [System.Diagnostics.Stopwatch]::StartNew()
& cargo build -p clmm-lp-api --bin clmm-lp-api --target-dir $env:CLMM_API_TARGET_DIR
if ($LASTEXITCODE -ne 0) {
    Write-Error "[Start-ClmmApi-8081] cargo build failed (exit $LASTEXITCODE). Fix compile errors before starting dashboard."
    exit $LASTEXITCODE
}
Write-Host ("[Start-ClmmApi-8081] Build OK in {0}s." -f [int]$buildSw.Elapsed.TotalSeconds) -ForegroundColor Green

$apiExe = Join-Path $RepoRoot (Join-Path $env:CLMM_API_TARGET_DIR "debug\clmm-lp-api.exe")
if (-not (Test-Path -LiteralPath $apiExe)) {
    Write-Error "[Start-ClmmApi-8081] Missing binary after build: $apiExe"
    exit 1
}

# Run binary in a separate window (no second compile); keep it open on errors.
$runCmd = @"
& {
  `$ErrorActionPreference = 'Continue'
  `$env:API_PORT = '8081'
  `$env:CLMM_REPO_ROOT = '$RepoRoot'
  `$env:RUST_LOG = 'info'
  `$env:DRY_RUN = '$($env:DRY_RUN)'
  `$env:CLMM_API_TARGET_DIR = '$($env:CLMM_API_TARGET_DIR)'
  `$env:CLMM_LP_CLI_PATH = '$($env:CLMM_LP_CLI_PATH)'
  `$env:NPM_CONFIG_CACHE = '$($env:NPM_CONFIG_CACHE)'
  `$env:TEMP = '$($env:TEMP)'
  `$env:TMP = '$($env:TMP)'
  `$env:KEYPAIR_PATH = '$($env:KEYPAIR_PATH)'
  `$env:SOLANA_KEYPAIR_PATH = '$($env:SOLANA_KEYPAIR_PATH)'
  `$env:WALLET_KEYPAIR_PATH = '$($env:WALLET_KEYPAIR_PATH)'
  `$env:SOLANA_KEYPAIR = '$($env:SOLANA_KEYPAIR)'
  `$env:WALLET_KEYPAIR_BASE58 = '$($env:WALLET_KEYPAIR_BASE58)'
  `$env:CLMM_STRANDED_RECONCILE_INTERVAL_SECS = '$($env:CLMM_STRANDED_RECONCILE_INTERVAL_SECS)'
  `$env:CLMM_IL_LEDGER_PATH = '$($env:CLMM_IL_LEDGER_PATH)'
  `$env:CLMM_PENDING_OPEN_RECOVERY_PATH = '$($env:CLMM_PENDING_OPEN_RECOVERY_PATH)'
  `$env:DATABASE_URL = '$($env:DATABASE_URL)'
  `$env:DATABASE_POOL_SIZE = '$($env:DATABASE_POOL_SIZE)'
  Write-Host '[clmm-lp-api] logging to: $logPath' -ForegroundColor DarkGray
  Write-Host '[clmm-lp-api] exe: $apiExe' -ForegroundColor DarkGray
  & '$apiExe' 2>&1 | Tee-Object -FilePath '$logPath'
}
"@

Start-Process -FilePath "pwsh" `
  -ArgumentList @("-NoProfile", "-NoExit", "-ExecutionPolicy", "Bypass", "-Command", $runCmd) `
  -WorkingDirectory $RepoRoot `
  -WindowStyle Normal

Write-Host "[Start-ClmmApi-8081] OK. API should be reachable at http://127.0.0.1:8081/api/v1/health" -ForegroundColor Green
Write-Host "[Start-ClmmApi-8081] Logs: $logPath" -ForegroundColor DarkGray

