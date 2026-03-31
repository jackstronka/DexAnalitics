# Fast open->close: start close immediately after `position PDA:` appears.
# This avoids waiting for open's post-tx ledger enrichment (getTransaction retries),
# which can add tens of seconds on public RPC.
#
# Measures confirm->confirm using timestamps from `Transaction confirmed signature=...` log lines.
#
# Usage (repo root):
#   .\tools\orca_position_open_then_close_fast.ps1 -SleepSecs 0 -Verify
#

param(
  [string] $Pool = "Czfq3xZZDmsdGdUyrNLtRhGc47cXcZtLG4crryfu44zE",
  [string] $Keypair = "",
  [double] $RangeWidthPct = 10,
  [UInt64] $AmountA = 1000000,
  [UInt64] $AmountB = 1000,
  [UInt16] $SlippageBps = 50,
  # Close: Orca min-token-out slippage (higher avoids Whirlpool 6018 TokenMinSubceeded on tiny/fast cycles).
  [UInt16] $CloseSlippageBps = 500,
  # Prefer `cargo run` even if `target/*/clmm-lp-cli.exe` exists (useful after code changes).
  [switch] $ForceCargo,
  [UInt64] $SleepSecs = 0,
  [UInt64] $ReserveSolLamports = 15000000,
  [switch] $AutoFund,
  [UInt32] $AutoFundMaxRounds = 8,
  [UInt16] $FundSwapSlippageBps = 150,
  [UInt32] $FundDeficitBufferBps = 100,
  [switch] $SkipPreflight,
  [switch] $Verify,
  [switch] $Usd
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Info([string]$msg) { Write-Host ("[orca-open-close-fast] " + $msg) }
function Fail([string]$msg) { throw ("[orca-open-close-fast] " + $msg) }

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
Set-Location $repoRoot

if (-not $env:SOLANA_RPC_URL -or [string]::IsNullOrWhiteSpace($env:SOLANA_RPC_URL)) {
  $mainnetEnv = Join-Path $PSScriptRoot "mainnet_rpc_env.ps1"
  if (Test-Path -LiteralPath $mainnetEnv) {
    Info ("SOLANA_RPC_URL not set; dot-sourcing " + $mainnetEnv)
    . $mainnetEnv
  }
}
if (-not $env:SOLANA_RPC_URL -or [string]::IsNullOrWhiteSpace($env:SOLANA_RPC_URL)) {
  Fail "Missing SOLANA_RPC_URL. Dot-source tools/mainnet_rpc_env.ps1 first or export SOLANA_RPC_URL."
}

. (Join-Path $PSScriptRoot "clmm_rpc_tools_helpers.ps1")
if (Initialize-ClmmToolsRpcEnv) {
  Info "Default CLMM_RPC_DENYLIST=ankr,projectserum (mainnet fallbacks). Set CLMM_RPC_DENYLIST yourself to override."
}

if ([string]::IsNullOrWhiteSpace($Keypair)) {
  if ($env:KEYPAIR_PATH -and -not [string]::IsNullOrWhiteSpace($env:KEYPAIR_PATH)) {
    $Keypair = $env:KEYPAIR_PATH
  } elseif ($env:SOLANA_KEYPAIR_PATH -and -not [string]::IsNullOrWhiteSpace($env:SOLANA_KEYPAIR_PATH)) {
    $Keypair = $env:SOLANA_KEYPAIR_PATH
  } else {
    $Keypair = Join-Path $env:USERPROFILE ".config\solana\clmm_lp_bot_mainnet.json"
  }
}
if (-not (Test-Path -LiteralPath $Keypair)) {
  Fail ("Keypair file not found: " + $Keypair)
}

function ResolveOwnerFromKeypair([string]$kpPath) {
  if (Get-Command "solana-keygen" -ErrorAction SilentlyContinue) {
    return (& solana-keygen pubkey $kpPath | Select-Object -First 1).Trim()
  }
  $kpJson = Get-Content -LiteralPath $kpPath -Raw | ConvertFrom-Json
  if ($null -eq $kpJson) { throw "Could not parse keypair JSON at $kpPath" }
  $bytes = [byte[]]($kpJson | ForEach-Object { [byte]$_ })
  if ($bytes.Length -lt 64) { throw "Keypair JSON too short (expected 64 bytes), len=" + $bytes.Length }
  $pubkeyBytes = $bytes[32..63]

  function Base58Encode([byte[]] $data) {
    $alphabet = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz".ToCharArray()
    $zeroCount = 0
    for ($i = 0; $i -lt $data.Length; $i++) { if ($data[$i] -ne 0) { break }; $zeroCount++ }
    $digits = New-Object System.Collections.Generic.List[int]
    for ($i = 0; $i -lt $data.Length; $i++) {
      $carry = [int]$data[$i]
      for ($j = 0; $j -lt $digits.Count; $j++) {
        $carry += $digits[$j] * 256
        $digits[$j] = [int]($carry % 58)
        $carry = [int]([Math]::Floor($carry / 58))
      }
      while ($carry -gt 0) { $digits.Add([int]($carry % 58)); $carry = [int]([Math]::Floor($carry / 58)) }
    }
    $sb = New-Object System.Text.StringBuilder
    for ($i = 0; $i -lt $zeroCount; $i++) { [void]$sb.Append('1') }
    for ($i = $digits.Count - 1; $i -ge 0; $i--) { [void]$sb.Append($alphabet[$digits[$i]]) }
    return $sb.ToString()
  }
  return (Base58Encode $pubkeyBytes)
}

$owner = ResolveOwnerFromKeypair $Keypair
Info ("RPC=" + $env:SOLANA_RPC_URL)
Info ("Pool=" + $Pool)
Info ("Keypair=" + $Keypair)
Info ("Owner=" + $owner)
Info ("range_width_pct=" + $RangeWidthPct + " amount_a=" + $AmountA + " amount_b=" + $AmountB + " open_slippage_bps=" + $SlippageBps + " close_slippage_bps=" + $CloseSlippageBps + " sleep_secs=" + $SleepSecs)

$preferExe = -not $ForceCargo.IsPresent

if ($AutoFund.IsPresent) {
  Info ("Auto-fund: exact-out swaps on pool (max_rounds=" + $AutoFundMaxRounds + " swap_slippage_bps=" + $FundSwapSlippageBps + " deficit_buffer_bps=" + $FundDeficitBufferBps + ")...")
  . (Join-Path $PSScriptRoot "orca_position_preflight_core.ps1")
  try {
    Invoke-OrcaPositionAutoFundFromPool -Pool $Pool -RepoRoot $repoRoot -Owner $owner -Keypair $Keypair `
      -AmountA $AmountA -AmountB $AmountB -ReserveSolLamports $ReserveSolLamports `
      -PreferReleaseExe $preferExe -MaxRounds $AutoFundMaxRounds -SwapSlippageBps $FundSwapSlippageBps `
      -DeficitBufferBps $FundDeficitBufferBps -Quiet:$false
  } catch {
    Fail $_.Exception.Message
  }
}

if (-not $SkipPreflight.IsPresent) {
  Info "Running position-open preflight (orca-pool-read + wallet JSON)..."
  $preflightPath = Join-Path $PSScriptRoot "orca_position_open_preflight.ps1"
  $pfArgs = @{
    Pool               = $Pool
    Keypair            = $Keypair
    Owner              = $owner
    AmountA            = $AmountA
    AmountB            = $AmountB
    ReserveSolLamports = $ReserveSolLamports
  }
  if ($ForceCargo.IsPresent) { $pfArgs.CargoOnly = $true }
  & $preflightPath @pfArgs
  if ($LASTEXITCODE -ne 0) {
    Fail ("Position open preflight failed (exit " + $LASTEXITCODE + "). Fund wallet or swap (tools/orca_swap.ps1); or pass -SkipPreflight if you accept blind open.")
  }
} else {
  Info "Skipping position-open preflight (-SkipPreflight)."
}

function ResolveCliCommand([string]$repoRoot, [bool]$forceCargo) {
  if ($forceCargo) { return $null }
  $rel = Join-Path $repoRoot "target\\release\\clmm-lp-cli.exe"
  if (Test-Path -LiteralPath $rel) { return $rel }
  $dbg = Join-Path $repoRoot "target\\debug\\clmm-lp-cli.exe"
  if (Test-Path -LiteralPath $dbg) { return $dbg }
  return $null
}

$cliExe = ResolveCliCommand $repoRoot ([bool]$ForceCargo.IsPresent)

function Start-CliProcess([string]$step, [string[]]$argv, [string]$repoRoot, [string]$cliExe) {
  $p = New-Object System.Diagnostics.Process
  $p.StartInfo = New-Object System.Diagnostics.ProcessStartInfo
  $p.StartInfo.WorkingDirectory = $repoRoot
  $p.StartInfo.RedirectStandardOutput = $true
  $p.StartInfo.RedirectStandardError = $true
  $p.StartInfo.UseShellExecute = $false
  $p.StartInfo.CreateNoWindow = $true

  # If `$cliExe` is typed as [string], passing `$null` can coerce to empty string.
  if (-not [string]::IsNullOrWhiteSpace($cliExe)) {
    $p.StartInfo.FileName = $cliExe
    $p.StartInfo.Arguments = ($argv -join " ")
  } else {
    # Fallback to cargo run (slower, but works everywhere).
    $p.StartInfo.FileName = "cargo"
    $p.StartInfo.Arguments = ("run -p clmm-lp-cli --bin clmm-lp-cli -- " + ($argv -join " "))
  }

  Info ("Starting " + $step + ": " + $p.StartInfo.FileName + " " + $p.StartInfo.Arguments)
  $ok = $p.Start()
  if (-not $ok) { Fail ("Failed to start process for " + $step) }
  return $p
}

function ParseConfirmedTs([string]$line) {
  # Example: 2026-03-30T21:23:30.455760Z  INFO ... Transaction confirmed signature=...
  if ($line -match '^(\\d{4}-\\d{2}-\\d{2}T\\d{2}:\\d{2}:\\d{2}\\.\\d+Z)') {
    try { return [DateTime]::Parse($Matches[1]).ToUniversalTime() } catch { return $null }
  }
  return $null
}

function TryParsePositionPda([string]$line) {
  if ([string]::IsNullOrWhiteSpace($line)) { return $null }
  # Prefer simple substring parsing over regex to survive ANSI/control characters.
  $markers = @("position PDA:", "opened position PDA:")
  foreach ($m in $markers) {
    $idx = $line.IndexOf($m, [System.StringComparison]::OrdinalIgnoreCase)
    if ($idx -lt 0) { continue }
    $after = $line.Substring($idx + $m.Length).Trim()
    if ([string]::IsNullOrWhiteSpace($after)) { continue }
    $tok = ($after.Split(" ", [System.StringSplitOptions]::RemoveEmptyEntries) | Select-Object -First 1)
    if (-not [string]::IsNullOrWhiteSpace($tok)) { return $tok }
  }
  return $null
}

function TryParseSignatureLoose([string]$line) {
  if ([string]::IsNullOrWhiteSpace($line)) { return $null }
  # Handles: "signature: <SIG>" and "open signature: <SIG>"
  $idx = $line.IndexOf("signature:", [System.StringComparison]::OrdinalIgnoreCase)
  if ($idx -lt 0) { return $null }
  $after = $line.Substring($idx + 10).Trim()
  if ([string]::IsNullOrWhiteSpace($after)) { return $null }
  return ($after.Split(" ", [System.StringSplitOptions]::RemoveEmptyEntries) | Select-Object -First 1)
  return $null
}

$openArgs = @(
  "orca-position-open",
  "--pool", $Pool,
  "--keypair", $Keypair,
  "--range-width-pct", ([string]$RangeWidthPct),
  "--amount-a", ([string]$AmountA),
  "--amount-b", ([string]$AmountB),
  "--slippage-bps", ([string]$SlippageBps)
)

$openProc = Start-CliProcess -step "open" -argv $openArgs -repoRoot $repoRoot -cliExe $cliExe

$openPda = $null
$openSig = $null
$openConfirmedAt = $null

$closeProc = $null
$closeSig = $null
$closeConfirmedAt = $null

# Read open output; once we have PDA (and optionally have waited sleep), start close immediately.
while (-not $openProc.HasExited) {
  while (-not $openProc.StandardOutput.EndOfStream) {
    $line = $openProc.StandardOutput.ReadLine()
    if ($null -eq $line) { break }
    Write-Host $line

    if ($null -eq $openConfirmedAt -and $line -match 'Transaction confirmed\\s+signature=') {
      $ts = ParseConfirmedTs $line
      if ($null -ne $ts) { $openConfirmedAt = $ts }
    }
    if ($null -eq $openPda) { $p = TryParsePositionPda $line; if ($p) { $openPda = $p } }
    if ($null -eq $openSig) { $s = TryParseSignatureLoose $line; if ($s) { $openSig = $s } }

    if ($null -eq $closeProc -and -not [string]::IsNullOrWhiteSpace($openPda)) {
      if ($SleepSecs -gt 0) {
        Info ("Sleeping " + $SleepSecs + "s before close...")
        Start-Sleep -Seconds $SleepSecs
      }
      $closeArgs = @(
        "orca-position-close",
        "--position", $openPda,
        "--keypair", $Keypair,
        "--slippage-bps", ([string]$CloseSlippageBps)
      )
      $closeProc = Start-CliProcess -step "close" -argv $closeArgs -repoRoot $repoRoot -cliExe $cliExe
      Info ("Close started for position " + $openPda)
    }
  }

  while (-not $openProc.StandardError.EndOfStream) {
    $line = $openProc.StandardError.ReadLine()
    if ($null -eq $line) { break }
    Write-Host $line
    if ($null -eq $openConfirmedAt -and $line -match 'Transaction confirmed\\s+signature=') {
      $ts = ParseConfirmedTs $line
      if ($null -ne $ts) { $openConfirmedAt = $ts }
    }
    if ($null -eq $openPda) { $p = TryParsePositionPda $line; if ($p) { $openPda = $p } }
    if ($null -eq $openSig) { $s = TryParseSignatureLoose $line; if ($s) { $openSig = $s } }
  }

  Start-Sleep -Milliseconds 50
}

# Drain remaining open output (and keep parsing PDA/signature).
while (-not $openProc.StandardOutput.EndOfStream) {
  $line = $openProc.StandardOutput.ReadLine()
  if ($null -eq $line) { break }
  Write-Host $line
  if ($null -eq $openPda) { $p = TryParsePositionPda $line; if ($p) { $openPda = $p } }
  if ($null -eq $openSig) { $s = TryParseSignatureLoose $line; if ($s) { $openSig = $s } }
  if ($null -eq $openConfirmedAt -and $line -match 'Transaction confirmed\\s+signature=') {
    $ts = ParseConfirmedTs $line
    if ($null -ne $ts) { $openConfirmedAt = $ts }
  }
}
while (-not $openProc.StandardError.EndOfStream) {
  $line = $openProc.StandardError.ReadLine()
  if ($null -eq $line) { break }
  Write-Host $line
  if ($null -eq $openPda) { $p = TryParsePositionPda $line; if ($p) { $openPda = $p } }
  if ($null -eq $openSig) { $s = TryParseSignatureLoose $line; if ($s) { $openSig = $s } }
  if ($null -eq $openConfirmedAt -and $line -match 'Transaction confirmed\\s+signature=') {
    $ts = ParseConfirmedTs $line
    if ($null -ne $ts) { $openConfirmedAt = $ts }
  }
}

# If open exited quickly, PDA may only appear in drain; start close now if needed.
if ($null -eq $closeProc -and -not [string]::IsNullOrWhiteSpace($openPda)) {
  if ($SleepSecs -gt 0) {
    Info ("Sleeping " + $SleepSecs + "s before close...")
    Start-Sleep -Seconds $SleepSecs
  }
  $closeArgs = @(
    "orca-position-close",
    "--position", $openPda,
    "--keypair", $Keypair,
    "--slippage-bps", ([string]$CloseSlippageBps)
  )
  $closeProc = Start-CliProcess -step "close" -argv $closeArgs -repoRoot $repoRoot -cliExe $cliExe
  Info ("Close started for position " + $openPda)
}

if ($null -eq $closeProc) {
  Fail "Open finished but close was never started (could not parse position PDA)."
}

# Read close output until exit
while (-not $closeProc.HasExited) {
  while (-not $closeProc.StandardOutput.EndOfStream) {
    $line = $closeProc.StandardOutput.ReadLine()
    if ($null -eq $line) { break }
    Write-Host $line
    if ($null -eq $closeConfirmedAt -and $line -match 'Transaction confirmed\\s+signature=') {
      $ts = ParseConfirmedTs $line
      if ($null -ne $ts) { $closeConfirmedAt = $ts }
    }
    if ($null -eq $closeSig -and $line -match 'signature:\\s*([1-9A-HJ-NP-Za-km-z]{80,})') {
      $closeSig = $Matches[1]
    }
  }
  while (-not $closeProc.StandardError.EndOfStream) {
    $line = $closeProc.StandardError.ReadLine()
    if ($null -eq $line) { break }
    Write-Host $line
    if ($null -eq $closeConfirmedAt -and $line -match 'Transaction confirmed\\s+signature=') {
      $ts = ParseConfirmedTs $line
      if ($null -ne $ts) { $closeConfirmedAt = $ts }
    }
    if ($null -eq $closeSig -and $line -match 'signature:\\s*([1-9A-HJ-NP-Za-km-z]{80,})') {
      $closeSig = $Matches[1]
    }
  }
  Start-Sleep -Milliseconds 50
}

while (-not $closeProc.StandardOutput.EndOfStream) { Write-Host ($closeProc.StandardOutput.ReadLine()) }
while (-not $closeProc.StandardError.EndOfStream) { Write-Host ($closeProc.StandardError.ReadLine()) }

if ($null -ne $openConfirmedAt -and $null -ne $closeConfirmedAt) {
  $delta = $closeConfirmedAt - $openConfirmedAt
  Info ("confirm->confirm: " + [math]::Round($delta.TotalMilliseconds) + " ms (" + [math]::Round($delta.TotalSeconds, 3) + " s)")
} else {
  Info "confirm->confirm: (missing confirmed timestamps; check logs)"
}

if ($openSig) { Info ("open signature:  " + $openSig) }
if ($closeSig) { Info ("close signature: " + $closeSig) }

# Cost summary (best-effort) from lifecycle ledger
function Resolve-LedgerPath([string] $RepoRoot) {
  if ($env:CLMM_POSITION_LIFECYCLE_LEDGER_PATH -and -not [string]::IsNullOrWhiteSpace($env:CLMM_POSITION_LIFECYCLE_LEDGER_PATH)) {
    return $env:CLMM_POSITION_LIFECYCLE_LEDGER_PATH
  }
  if ($env:CLMM_POSITION_OPEN_LEDGER_PATH -and -not [string]::IsNullOrWhiteSpace($env:CLMM_POSITION_OPEN_LEDGER_PATH)) {
    return $env:CLMM_POSITION_OPEN_LEDGER_PATH
  }
  return (Join-Path $RepoRoot "data\\ledger\\orca_position_lifecycle.jsonl")
}
function Find-LedgerRowBySignature([string] $LedgerPath, [string] $Signature) {
  if (-not (Test-Path -LiteralPath $LedgerPath)) { return $null }
  $lines = Get-Content -LiteralPath $LedgerPath
  $needle = '"signature":"' + $Signature + '"'
  for ($i = $lines.Count - 1; $i -ge 0; $i--) {
    $line = $lines[$i]
    if ($null -eq $line) { continue }
    if ($line.IndexOf($needle, [System.StringComparison]::Ordinal) -lt 0) { continue }
    return ($line | ConvertFrom-Json)
  }
  return $null
}

$ledgerPath = Resolve-LedgerPath $repoRoot
$openRow = if ($openSig) { Find-LedgerRowBySignature $ledgerPath $openSig } else { $null }
$closeRow = if ($closeSig) { Find-LedgerRowBySignature $ledgerPath $closeSig } else { $null }
if ($null -ne $openRow -and $null -ne $closeRow) {
  $openFee = [UInt64]$openRow.tx_fee_lamports
  $closeFee = [UInt64]$closeRow.tx_fee_lamports
  $totalFeeLamports = $openFee + $closeFee
  $totalFeeSol = [double]$totalFeeLamports / 1000000000.0
  $netOpen = [Int64]$openRow.fee_payer_net_lamports_delta
  $netClose = [Int64]$closeRow.fee_payer_net_lamports_delta
  $netLamports = $netOpen + $netClose
  $netSol = [double]$netLamports / 1000000000.0

  Write-Host ""
  Write-Host "=== Cost (from ledger) ==="
  Write-Host ("Ledger: " + $ledgerPath)
  Write-Host ("Network fee: " + $totalFeeLamports + " lamports (" + $totalFeeSol + " SOL)")
  Write-Host ("Net SOL delta (fee payer): " + $netLamports + " lamports (" + $netSol + " SOL)")
  if ($Usd) {
    try {
      $solUsd = (Invoke-RestMethod -Uri "https://api.coingecko.com/api/v3/simple/price?ids=solana&vs_currencies=usd" -TimeoutSec 15).solana.usd
      Write-Host ("SOL/USD (CoinGecko): " + $solUsd)
      Write-Host ("Fee USD: " + ([math]::Round($totalFeeSol * [double]$solUsd, 6)))
      Write-Host ("Net USD: " + ([math]::Round($netSol * [double]$solUsd, 6)))
    } catch {
      Write-Host "USD conversion skipped (CoinGecko fetch failed)."
    }
  }
} else {
  Write-Host ""
  Write-Host "Ledger cost not found yet (RPC getTransaction can lag). You can re-run later to query by signatures."
}

if ($Verify) {
  Info "Verifying on-chain entries=0..."
  if ($null -ne $cliExe) {
    & $cliExe orca-positions-list --owner $owner --keypair $Keypair
  } else {
    & cargo run -p clmm-lp-cli --bin clmm-lp-cli -- orca-positions-list --owner $owner --keypair $Keypair
  }
}

