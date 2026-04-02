# Print three ready-to-run commands: whETH/SOL pool Hktf…, ticks [-55416,-55216] ≈ 25.0–25.5 SOL per whETH.
# Strategies: A oor_recenter, B periodic 12h, C threshold 5% (JSON under data/experiments/wheth-sol-manual-range-25-25p5/).
#
# Usage: . .\tools\mainnet_rpc_env.ps1
#        powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\wheth_sol_three_bots_manual_range_25_25p5.ps1
#
param(
  [string] $Keypair = "",
  [UInt64] $AmountA = 1000000000,
  [UInt64] $AmountB = 100000000,
  [UInt16] $SlippageBps = 100,
  [UInt32] $EvalIntervalSecs = 300,
  [UInt32] $PollIntervalSecs = 30
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$Pool = "HktfL7iwGKT5QHjywQkcDnZXScoh811k7akrMZJkCcEF"
$Base = Join-Path $repoRoot "data\experiments\wheth-sol-manual-range-25-25p5"
if (-not (Test-Path $Base)) { throw "Missing $Base" }

if ([string]::IsNullOrWhiteSpace($Keypair)) {
  $Keypair = if ($env:KEYPAIR_PATH) { $env:KEYPAIR_PATH } else { Join-Path $env:USERPROFILE ".config\solana\clmm_lp_bot_mainnet.json" }
}

$exe = Join-Path $repoRoot "target\release\clmm-lp-cli.exe"
if (-not (Test-Path $exe)) { $exe = Join-Path $repoRoot "target\debug\clmm-lp-cli.exe" }
$run = if (Test-Path $exe) { "& `"$exe`"" } else { "cargo run -p clmm-lp-cli --bin clmm-lp-cli --" }

function Line([string]$label, [string]$jsonName, [string]$IlLedgerName) {
  $json = Join-Path $Base $jsonName
  $j = (Resolve-Path $json).Path
  $ilPath = Join-Path $Base $IlLedgerName
  Write-Host ""
  Write-Host "# $label" -ForegroundColor Cyan
  Write-Host @"
$run orca-bot-open-and-run `
  --pool $Pool `
  --tick-lower=-55416 `
  --tick-upper=-55216 `
  --amount-a $AmountA `
  --amount-b $AmountB `
  --slippage-bps $SlippageBps `
  --keypair `"$Keypair`" `
  --optimize-result-json `"$j`" `
  --il-ledger-path `"$ilPath`" `
  --eval-interval-secs $EvalIntervalSecs `
  --poll-interval-secs $PollIntervalSecs
"@
  Write-Host '# Rebalance tx in bot loop: append --execute (open tx always signs when keypair is set).'
  Write-Host "# IL/rebalance JSONL: $ilPath — set CLMM_IL_LEDGER_PATH to the same path on the API host to see rows in Bot activity / Position detail."
}

Write-Host "Tick range [-55416,-55216] = ~25.0 SOL/whETH .. ~25.5 SOL/whETH (verify vs orca-pool-read before mainnet)."
Line 'Bot A oor_recenter' 'winner-A-oor_recenter.json' 'il_ledger_bot_A.jsonl'
Line 'Bot B periodic 12h' 'winner-B-periodic.json' 'il_ledger_bot_B.jsonl'
Line 'Bot C threshold 5pct' 'winner-C-threshold.json' 'il_ledger_bot_C.jsonl'
Write-Host ''
Write-Host 'Run each command in a separate terminal. See doc/WHETH_SOL_THREE_BOTS_FIRST_RUN.md - dry-run first.'
