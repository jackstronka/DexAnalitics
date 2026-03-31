# Orca Whirlpool swap wrapper (PowerShell).
# Preflight: prints current wallet state (SOL + SPL) using tools/solana_account_state.ps1
# Executes: `clmm-lp-cli orca-swap ...` (dry-run by default unless -Execute is set)
# Postflight: prints wallet state again + delta for specified mint and SOL.
#
# Usage (repo root):
#   . .\tools\mainnet_rpc_env.ps1
#   $kp = "$env:USERPROFILE\.config\solana\clmm_lp_bot_mainnet.json"
#   powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\orca_swap.ps1 `
#     -Owner 8s9BcTUTXmWmZVPDrkoMNKsU6n1dRsihySv1bSteSvMQ `
#     -Keypair $kp `
#     -Pool <WHIRLPOOL_ADDRESS> `
#     -SpecifiedMint So11111111111111111111111111111111111111112 `
#     -SwapType exact-in `
#     -AmountRaw 1000000 `
#     -SlippageBps 100 `
#     -Execute
#
# Note:
# - Amount is in base units for the specified mint (decimals depend on token).
# - For SOL, specified mint is wSOL: So11111111111111111111111111111111111111112
# - Curated 3-pair wrapper (From/To + exact-in/out): tools/orca_swap_curated.ps1 -ListPairs

param(
    [Parameter(Mandatory = $true)]
    [string] $Owner,

    [Parameter(Mandatory = $true)]
    [string] $Keypair,

    [Parameter(Mandatory = $true)]
    [string] $Pool,

    [Parameter(Mandatory = $true)]
    [string] $SpecifiedMint,

    [ValidateSet("exact-in", "exact-out")]
    [string] $SwapType = "exact-in",

    # Amount in raw base units (per mint decimals).
    [UInt64] $AmountRaw = 0,

    # If set, call CLI with --dry-run=false (sign+send tx).
    [switch] $Execute,

    [UInt16] $SlippageBps = 100,

    # Force `cargo run` even when target\*\clmm-lp-cli.exe exists.
    [switch] $CargoOnly,

    # Optional override; by default uses release/debug exe if present, else cargo run (unless -CargoOnly).
    [string] $CliCommand = "",

    [string] $RepoRoot = ""
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Info([string]$msg) { Write-Host ("[orca-swap.ps1] " + $msg) }
function Fail([string]$msg) { throw ("[orca-swap.ps1] " + $msg) }

if ([string]::IsNullOrWhiteSpace($RepoRoot)) {
    if (-not $PSScriptRoot -or [string]::IsNullOrWhiteSpace($PSScriptRoot)) {
        Fail "RepoRoot not provided and PSScriptRoot is unavailable. Pass -RepoRoot <path-to-repo>."
    }
    $RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
}

Set-Location $RepoRoot

if (-not (Test-Path -LiteralPath $Keypair)) { Fail ("Keypair file not found: " + $Keypair) }
if ($AmountRaw -eq 0) { Fail "-AmountRaw must be > 0" }

if (-not $env:SOLANA_RPC_URL -or [string]::IsNullOrWhiteSpace($env:SOLANA_RPC_URL)) {
    Fail "Missing SOLANA_RPC_URL. Dot-source .\\tools\\mainnet_rpc_env.ps1 first (or set env var)."
}

. (Join-Path $PSScriptRoot "clmm_rpc_tools_helpers.ps1")
if (Initialize-ClmmToolsRpcEnv) {
    Info "Default CLMM_RPC_DENYLIST=ankr,projectserum for mainnet fallbacks (override with env)."
}

$preferExe = -not $CargoOnly.IsPresent

if (-not (Test-Path -LiteralPath (Join-Path $RepoRoot "Cargo.toml"))) {
    Fail "Repo root does not look correct (Cargo.toml missing)."
}

# Run CLI in a child process with redirected streams so cargo/rust stderr (e.g. "Finished `dev` profile...")
# does not become ErrorRecord / NativeCommandError in the caller — fixes nested -File scripts (e.g. orca_fund_cbbtc_usdc_open.ps1).
function Invoke-OrcaSwapNativeCli {
    param(
        [Parameter(Mandatory)][string]$FilePath,
        [Parameter(Mandatory)][string[]]$ArgumentList,
        [Parameter(Mandatory)][string]$WorkingDirectory
    )
    $outF = [System.IO.Path]::GetTempFileName()
    $errF = [System.IO.Path]::GetTempFileName()
    try {
        $proc = Start-Process -FilePath $FilePath -ArgumentList $ArgumentList -WorkingDirectory $WorkingDirectory `
            -Wait -PassThru -NoNewWindow `
            -RedirectStandardOutput $outF `
            -RedirectStandardError $errF
        $code = 1
        if ($null -ne $proc.ExitCode) { $code = [int]$proc.ExitCode }
        $outLines = @()
        $errLines = @()
        if (Test-Path -LiteralPath $outF) {
            $outLines = @(Get-Content -LiteralPath $outF -ErrorAction SilentlyContinue | ForEach-Object { "$_" })
        }
        if (Test-Path -LiteralPath $errF) {
            $errLines = @(Get-Content -LiteralPath $errF -ErrorAction SilentlyContinue | ForEach-Object { "$_" })
        }
        foreach ($line in $outLines) { Write-Host $line }
        foreach ($line in $errLines) { Write-Host $line }
        $all = New-Object System.Collections.Generic.List[string]
        foreach ($x in $outLines) { [void]$all.Add($x) }
        foreach ($x in $errLines) { [void]$all.Add($x) }
        return @{ ExitCode = $code; Lines = $all.ToArray() }
    }
    finally {
        Remove-Item -LiteralPath $outF -Force -ErrorAction SilentlyContinue
        Remove-Item -LiteralPath $errF -Force -ErrorAction SilentlyContinue
    }
}

function Read-State([string] $label) {
    Info ("Fetching account state (" + $label + ")")
    $json = & powershell -NoProfile -ExecutionPolicy Bypass -File (Join-Path $PSScriptRoot "solana_account_state.ps1") `
        -Owner $Owner -Json
    return ($json | ConvertFrom-Json)
}

function Index-TokensByMint($state) {
    $map = @{}
    foreach ($a in $state.spl_token_accounts) {
        $mint = [string]$a.mint
        if (-not $map.ContainsKey($mint)) {
            $map[$mint] = @()
        }
        $map[$mint] += $a
    }
    return $map
}

function Get-UiAmountForMint($state, [string] $mint) {
    $sum = 0.0
    $map = Index-TokensByMint $state
    if (-not $map.ContainsKey($mint)) { return 0.0 }
    foreach ($a in $map[$mint]) {
        if ($null -ne $a.ui_amount) { $sum += [double]$a.ui_amount }
    }
    return $sum
}

$pre = Read-State "pre"

$dryRun = -not $Execute.IsPresent
Info ("Running swap (dry_run=" + $dryRun + ")")

$cliArgs = @(
    "orca-swap",
    "--pool", $Pool,
    "--specified-mint", $SpecifiedMint,
    "--swap-type", $SwapType,
    "--amount", ([string]$AmountRaw),
    "--slippage-bps", ([string]$SlippageBps),
    "--keypair", $Keypair
)
if ($dryRun) { $cliArgs += @("--dry-run") }

$inv = $null
if (-not [string]::IsNullOrWhiteSpace($CliCommand)) {
    $inv = Invoke-OrcaSwapNativeCli -FilePath $CliCommand -ArgumentList ([string[]]$cliArgs) -WorkingDirectory $RepoRoot
} else {
    $exe = if ($preferExe) { Resolve-ClmmLpCliExe $RepoRoot } else { $null }
    if (-not [string]::IsNullOrWhiteSpace($exe)) {
        $inv = Invoke-OrcaSwapNativeCli -FilePath $exe -ArgumentList ([string[]]$cliArgs) -WorkingDirectory $RepoRoot
    } else {
        $cargoCmd = Get-Command "cargo" -ErrorAction SilentlyContinue
        if (-not $cargoCmd) { Fail "cargo not found in PATH (required when no clmm-lp-cli.exe)." }
        [string[]]$cargoArgs = @("run", "-p", "clmm-lp-cli", "--bin", "clmm-lp-cli", "--") + [string[]]$cliArgs
        $inv = Invoke-OrcaSwapNativeCli -FilePath $cargoCmd.Source -ArgumentList $cargoArgs -WorkingDirectory $RepoRoot
    }
}
if ($inv.ExitCode -ne 0) {
    Fail ("clmm-lp-cli orca-swap failed (exit " + $inv.ExitCode + ").")
}

$sig = $null
foreach ($line in $inv.Lines) {
    # Single-quoted pattern so \s is regex whitespace (in double quotes "\s" is literal backslash-s).
    if ($line -match 'signature:\s*([1-9A-HJ-NP-Za-km-z]{80,})') {
        $sig = $Matches[1]
    }
}

if ($dryRun) {
    Info "Dry-run finished (no on-chain tx sent)."
} else {
    if (-not $sig) { Info "Swap executed but signature not parsed from output (check console log above)." }
    else { Info ("Signature: " + $sig) }
}

$post = Read-State "post"

$preSol = [double]$pre.native_sol.sol
$postSol = [double]$post.native_sol.sol
$dSol = [math]::Round(($postSol - $preSol), 9)

$preSpecified = Get-UiAmountForMint $pre $SpecifiedMint
$postSpecified = Get-UiAmountForMint $post $SpecifiedMint
$dSpecified = [math]::Round(($postSpecified - $preSpecified), 12)

Write-Host ""
Write-Host "=== Delta (post - pre) ==="
Write-Host ("SOL:            " + $dSol)
Write-Host ("Specified mint:  " + $SpecifiedMint + "  " + $dSpecified)

if ($sig) {
    Write-Host ("Tx: https://solscan.io/tx/" + $sig)
}

