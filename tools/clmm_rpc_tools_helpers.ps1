# Shared helpers for tools/*.ps1 that invoke clmm-lp-cli with RpcProvider defaults.
# Dot-source after SOLANA_RPC_URL is set.

function Initialize-ClmmToolsRpcEnv {
  <#
  If CLMM_RPC_DENYLIST is unset/empty and primary RPC looks like mainnet, set a denylist so
  built-in RpcConfig fallbacks skip substrings that often break automation (403 on public Ankr,
  legacy Project Serum hostname).
  Rust: crates/protocols/src/rpc/config.rs filters fallback URLs by substring.
  Override: $env:CLMM_RPC_DENYLIST = ''  (empty = no denylist) or your own comma-separated substrings.
  #>
  if ($env:CLMM_RPC_DENYLIST -and $env:CLMM_RPC_DENYLIST.Trim().Length -gt 0) {
    return $false
  }
  $u = $env:SOLANA_RPC_URL
  if (-not $u -or [string]::IsNullOrWhiteSpace($u)) { return $false }
  if ($u -match 'devnet') { return $false }
  $env:CLMM_RPC_DENYLIST = 'ankr,projectserum'
  return $true
}

function Resolve-ClmmLpCliExe {
  param([Parameter(Mandatory)][string]$RepoRoot)
  $release = Join-Path $RepoRoot "target\release\clmm-lp-cli.exe"
  if (Test-Path -LiteralPath $release) { return $release }
  $debug = Join-Path $RepoRoot "target\debug\clmm-lp-cli.exe"
  if (Test-Path -LiteralPath $debug) { return $debug }
  return $null
}

function Invoke-ClmmLpCliStream {
  param(
    [Parameter(Mandatory)][string]$RepoRoot,
    [bool]$PreferReleaseExe = $true,
    [Parameter(Mandatory)][string[]]$Argv,
    [Parameter(Mandatory)][scriptblock]$OnLine,
    [Parameter(Mandatory)][string]$StepLabel
  )
  $exe = if ($PreferReleaseExe) { Resolve-ClmmLpCliExe $RepoRoot } else { $null }
  $oldEap = $ErrorActionPreference
  try {
    $ErrorActionPreference = "Continue"
    if (-not [string]::IsNullOrWhiteSpace($exe)) {
      & $exe @Argv 2>&1 | ForEach-Object { $line = "$_"; & $OnLine $line; $line }
    } else {
      $cargoArgs = @("run", "-p", "clmm-lp-cli", "--bin", "clmm-lp-cli", "--") + $Argv
      & cargo @cargoArgs 2>&1 | ForEach-Object { $line = "$_"; & $OnLine $line; $line }
    }
    $code = $LASTEXITCODE
  } finally {
    $ErrorActionPreference = $oldEap
  }
  if ($code -ne 0) {
    throw "[$StepLabel] failed (exit=$code)"
  }
}

function Invoke-ClmmLpCliCapture {
  param(
    [Parameter(Mandatory)][string]$RepoRoot,
    [bool]$PreferReleaseExe = $true,
    [Parameter(Mandatory)][string[]]$Argv,
    [Parameter(Mandatory)][string]$StepLabel
  )
  $exe = if ($PreferReleaseExe) { Resolve-ClmmLpCliExe $RepoRoot } else { $null }
  $oldEap = $ErrorActionPreference
  try {
    $ErrorActionPreference = "Continue"
    if (-not [string]::IsNullOrWhiteSpace($exe)) {
      $out = & $exe @Argv 2>&1 | ForEach-Object { "$_" }
    } else {
      $cargoArgs = @("run", "-p", "clmm-lp-cli", "--bin", "clmm-lp-cli", "--") + $Argv
      $out = & cargo @cargoArgs 2>&1 | ForEach-Object { "$_" }
    }
    $code = $LASTEXITCODE
  } finally {
    $ErrorActionPreference = $oldEap
  }
  if ($code -ne 0) {
    foreach ($l in $out) { Write-Host $l }
    throw "[$StepLabel] failed (exit=$code)"
  }
  return $out
}
