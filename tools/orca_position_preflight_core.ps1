# Shared preflight + optional on-pool auto-funding (exact-out swaps) for Orca position open.
# Dot-source from tools/*.ps1 after $RepoRoot is known. No param() block.

Set-StrictMode -Version Latest

$script:OrcaPreflightWsol = "So11111111111111111111111111111111111111112"

$script:OrcaPreflightMintLabel = @{
  $script:OrcaPreflightWsol                          = "SOL/wSOL"
  "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v"     = "USDC"
  "7vfCXTUXx5WJV5JADk17DUJ4ksgau7utNKj4b963voxs"     = "whETH (portal)"
  "cbbtcf3aa214zXHbiAZQwf4122FBYbraNdFqgw4iMij"     = "cbBTC"
}

function Get-OrcaPreflightMintLabel([string]$m) {
  if ($script:OrcaPreflightMintLabel.ContainsKey($m)) { return $script:OrcaPreflightMintLabel[$m] }
  return $m
}

function Sum-OrcaPreflightSplRawForMint([object]$state, [string]$mint) {
  $sum = [bigint]0
  foreach ($a in $state.spl_token_accounts) {
    if ([string]$a.mint -ne $mint) { continue }
    $sum += [bigint][string]$a.amount_raw
  }
  return $sum
}

function Get-OrcaPreflightSpendableRaw {
  param(
    [object]$State,
    [string]$Mint,
    [bigint]$NativeLamports
  )
  if ($Mint -eq $script:OrcaPreflightWsol) {
    return $NativeLamports + (Sum-OrcaPreflightSplRawForMint $State $script:OrcaPreflightWsol)
  }
  return (Sum-OrcaPreflightSplRawForMint $State $Mint)
}

function Parse-OrcaPreflightPoolReadMints([string[]]$Lines) {
  $ma = $null
  $mb = $null
  foreach ($line in $Lines) {
    if ($line -match '^\s*token_mint_a:\s*(\S+)\s*$') { $ma = $Matches[1].Trim() }
    if ($line -match '^\s*token_mint_b:\s*(\S+)\s*$') { $mb = $Matches[1].Trim() }
  }
  if ([string]::IsNullOrWhiteSpace($ma) -or [string]::IsNullOrWhiteSpace($mb)) {
    throw "Could not parse token_mint_a / token_mint_b from orca-pool-read output."
  }
  return @{ MintA = $ma; MintB = $mb }
}

function Get-OrcaBufferedExactOutAmount {
  param(
    [Parameter(Mandatory)][bigint]$Deficit,
    [UInt32]$BufferBps = 100
  )
  if ($Deficit -le 0) { return [UInt64]0 }
  $num = $Deficit * [bigint](10000 + [int]$BufferBps)
  $den = [bigint]10000
  $amt = ($num + $den - [bigint]1) / $den
  if ($amt -gt [bigint][UInt64]::MaxValue) { throw "[auto-fund] Buffered exact-out amount exceeds UInt64." }
  $u = [UInt64]$amt
  if ($u -lt 1) { return [UInt64]1 }
  return $u
}

function Get-OrcaPositionOpenPreflightState {
  param(
    [Parameter(Mandatory)][string]$Pool,
    [Parameter(Mandatory)][string]$RepoRoot,
    [Parameter(Mandatory)][string]$Owner,
    [Parameter(Mandatory)][UInt64]$AmountA,
    [Parameter(Mandatory)][UInt64]$AmountB,
    [Parameter(Mandatory)][UInt64]$ReserveSolLamports,
    [bool]$PreferReleaseExe = $true,
    [bool]$Quiet = $false,
    [bool]$SkipInitRpcEnv = $false
  )

  . (Join-Path $RepoRoot "tools\clmm_rpc_tools_helpers.ps1")
  if (-not $SkipInitRpcEnv) {
    if (Initialize-ClmmToolsRpcEnv) {
      if (-not $Quiet) { Write-Host "[preflight] default CLMM_RPC_DENYLIST set for mainnet." }
    }
  }

  $readOut = Invoke-ClmmLpCliCapture -RepoRoot $RepoRoot -PreferReleaseExe $PreferReleaseExe `
    -Argv @("orca-pool-read", "--pool-address", $Pool) -StepLabel "orca-pool-read"
  $mints = Parse-OrcaPreflightPoolReadMints ($readOut | ForEach-Object { "$_" })

  $stateJson = & powershell -NoProfile -ExecutionPolicy Bypass -File (Join-Path $RepoRoot "tools\solana_account_state.ps1") `
    -Owner $Owner -Json 2>&1 | ForEach-Object { "$_" }
  $jsonLine = ($stateJson | Where-Object { $_.TrimStart().StartsWith("{") } | Select-Object -First 1)
  if (-not $jsonLine) { throw "Could not parse solana_account_state JSON." }
  $state = $jsonLine | ConvertFrom-Json

  $nativeLamports = [bigint][string]$state.native_sol.lamports
  $needA = [bigint]$AmountA
  $needB = [bigint]$AmountB

  $availA = Get-OrcaPreflightSpendableRaw -State $state -Mint $mints.MintA -NativeLamports $nativeLamports
  $availB = Get-OrcaPreflightSpendableRaw -State $state -Mint $mints.MintB -NativeLamports $nativeLamports

  $okFee = $nativeLamports -ge [bigint]$ReserveSolLamports
  $okA = $availA -ge $needA
  $okB = $availB -ge $needB
  $ok = $okFee -and $okA -and $okB

  if (-not $Quiet) {
    Write-Host ("[preflight] pool=" + $Pool)
    Write-Host ("[preflight] mint_a=" + $mints.MintA + " (" + (Get-OrcaPreflightMintLabel $mints.MintA) + ") need_raw=" + $AmountA + " avail_raw=" + $availA + " ok=" + $okA)
    Write-Host ("[preflight] mint_b=" + $mints.MintB + " (" + (Get-OrcaPreflightMintLabel $mints.MintB) + ") need_raw=" + $AmountB + " avail_raw=" + $availB + " ok=" + $okB)
    Write-Host ("[preflight] native_lamports=" + $nativeLamports + " reserve>=" + $ReserveSolLamports + " ok=" + $okFee)
  }

  return [pscustomobject]@{
    Pool               = $Pool
    MintA              = $mints.MintA
    MintB              = $mints.MintB
    NeedA              = $AmountA
    NeedB              = $AmountB
    AvailA             = $availA
    AvailB             = $availB
    NativeLamports     = $nativeLamports
    ReserveSolLamports = $ReserveSolLamports
    OkFee              = $okFee
    OkA                = $okA
    OkB                = $okB
    Ok                 = $ok
  }
}

function Test-OrcaPositionOpenPreflight {
  param(
    [Parameter(Mandatory)][string]$Pool,
    [Parameter(Mandatory)][string]$RepoRoot,
    [Parameter(Mandatory)][string]$Owner,
    [Parameter(Mandatory)][UInt64]$AmountA,
    [Parameter(Mandatory)][UInt64]$AmountB,
    [Parameter(Mandatory)][UInt64]$ReserveSolLamports,
    [bool]$PreferReleaseExe = $true,
    [bool]$Quiet = $false
  )

  $r = Get-OrcaPositionOpenPreflightState -Pool $Pool -RepoRoot $RepoRoot -Owner $Owner `
    -AmountA $AmountA -AmountB $AmountB -ReserveSolLamports $ReserveSolLamports `
    -PreferReleaseExe $PreferReleaseExe -Quiet:$Quiet -SkipInitRpcEnv:$false

  if ($r.Ok) {
    if (-not $Quiet) { Write-Host "[preflight] PASS" }
    return $true
  }

  $msg = @()
  if (-not $r.OkFee) {
    $msg += "Native SOL too low for fee/rent reserve: have $($r.NativeLamports) lamports, need >= $ReserveSolLamports (raise -ReserveSolLamports if you accept higher risk)."
  }
  if (-not $r.OkA) {
    $msg += ("Insufficient token A (" + (Get-OrcaPreflightMintLabel $r.MintA) + "): need_raw=$($r.NeedA) avail_raw=$($r.AvailA) - fund wallet or swap into this mint.")
  }
  if (-not $r.OkB) {
    $msg += ("Insufficient token B (" + (Get-OrcaPreflightMintLabel $r.MintB) + "): need_raw=$($r.NeedB) avail_raw=$($r.AvailB) - fund wallet or swap into this mint.")
  }
  $full = $msg -join " "
  if (-not $Quiet) {
    Write-Host "[preflight] FAIL"
    foreach ($m in $msg) { Write-Host ("[preflight]  - " + $m) }
    Write-Host "[preflight] Hint: use tools/orca_swap.ps1 (same pool + --specified-mint + -Execute) or CEX bridge, then re-run preflight."
  }
  throw $full
}

function Invoke-OrcaPositionAutoFundFromPool {
  <#
  .SYNOPSIS
  Repeated exact-out swaps on the same Whirlpool until preflight passes (token A/B only).
  Requires enough of the *other* leg to pay each swap; does not fix native SOL fee reserve.

  .NOTES
  Fixed order: while deficit on mint A (pool token_mint_a), swap exact-out A (pay B); only then B.
  Plan wallet inventory accordingly — see doc/ORCA_RUNBOOK.md (Auto-fund / planowanie swapów).
  #>
  param(
    [Parameter(Mandatory)][string]$Pool,
    [Parameter(Mandatory)][string]$RepoRoot,
    [Parameter(Mandatory)][string]$Owner,
    [Parameter(Mandatory)][string]$Keypair,
    [Parameter(Mandatory)][UInt64]$AmountA,
    [Parameter(Mandatory)][UInt64]$AmountB,
    [Parameter(Mandatory)][UInt64]$ReserveSolLamports,
    [bool]$PreferReleaseExe = $true,
    [UInt32]$MaxRounds = 8,
    [UInt16]$SwapSlippageBps = 150,
    [UInt32]$DeficitBufferBps = 100,
    [bool]$Quiet = $false
  )

  . (Join-Path $RepoRoot "tools\clmm_rpc_tools_helpers.ps1")
  $null = Initialize-ClmmToolsRpcEnv

  for ($round = 0; $round -lt $MaxRounds; $round++) {
    $r = Get-OrcaPositionOpenPreflightState -Pool $Pool -RepoRoot $RepoRoot -Owner $Owner `
      -AmountA $AmountA -AmountB $AmountB -ReserveSolLamports $ReserveSolLamports `
      -PreferReleaseExe $PreferReleaseExe -Quiet:$Quiet -SkipInitRpcEnv:$true

    if ($r.Ok) {
      if (-not $Quiet) { Write-Host ("[auto-fund] preflight OK (round " + $round + ")") }
      return
    }

    if (-not $r.OkFee) {
      throw ("[auto-fund] Native SOL below fee reserve (" + $r.NativeLamports + " lamports < " + $ReserveSolLamports + "). Add SOL; swaps cannot raise the fee reserve check.")
    }

    $defA = [bigint]$r.NeedA - [bigint]$r.AvailA
    $defB = [bigint]$r.NeedB - [bigint]$r.AvailB
    if ($defA -lt 0) { $defA = 0 }
    if ($defB -lt 0) { $defB = 0 }

    if ($defA -eq 0 -and $defB -eq 0) {
      throw "[auto-fund] Preflight failed but token deficits are zero (unexpected)."
    }

    if (-not $Quiet) {
      Write-Host ("[auto-fund] round " + ($round + 1) + "/" + $MaxRounds + " deficit_a_raw=" + $defA + " deficit_b_raw=" + $defB)
      if ($defA -gt 0 -and $defB -gt 0) {
        Write-Host "[auto-fund] Both legs short: this tool always tops up mint A first (exact-out A, you pay B). Plan spare B accordingly; see doc/ORCA_RUNBOOK.md (planowanie swapów)."
      }
    }

    if ($defA -gt 0) {
      $raw = Get-OrcaBufferedExactOutAmount -Deficit $defA -BufferBps $DeficitBufferBps
      if (-not $Quiet) {
        Write-Host ("[auto-fund] orca-swap exact-out specified_mint=mint_a amount_raw=" + $raw + " mint=" + $r.MintA)
      }
      Invoke-ClmmLpCliCapture -RepoRoot $RepoRoot -PreferReleaseExe $PreferReleaseExe -Argv @(
        "orca-swap",
        "--pool", $Pool,
        "--specified-mint", $r.MintA,
        "--swap-type", "exact-out",
        "--amount", ([string]$raw),
        "--slippage-bps", ([string]$SwapSlippageBps),
        "--keypair", $Keypair
      ) -StepLabel "orca-swap auto-fund (mint A exact-out)"
      continue
    }

    if ($defB -gt 0) {
      $raw = Get-OrcaBufferedExactOutAmount -Deficit $defB -BufferBps $DeficitBufferBps
      if (-not $Quiet) {
        Write-Host ("[auto-fund] orca-swap exact-out specified_mint=mint_b amount_raw=" + $raw + " mint=" + $r.MintB)
      }
      Invoke-ClmmLpCliCapture -RepoRoot $RepoRoot -PreferReleaseExe $PreferReleaseExe -Argv @(
        "orca-swap",
        "--pool", $Pool,
        "--specified-mint", $r.MintB,
        "--swap-type", "exact-out",
        "--amount", ([string]$raw),
        "--slippage-bps", ([string]$SwapSlippageBps),
        "--keypair", $Keypair
      ) -StepLabel "orca-swap auto-fund (mint B exact-out)"
      continue
    }
  }

  throw ("[auto-fund] Exceeded MaxRounds=" + $MaxRounds + "; balances still insufficient for open. Fund manually or adjust amounts.")
}
