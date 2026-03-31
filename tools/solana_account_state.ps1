# Exact on-chain account snapshot for a Solana owner (pubkey): native SOL + SPL token accounts.
# Uses JSON-RPC only (no `solana` / `spl-token` CLI). Intended for automation and follow-up steps.
#
# Prereq: set SOLANA_RPC_URL (and optionally SOLANA_RPC_FALLBACK_URLS), e.g. dot-source:
#   . .\tools\mainnet_rpc_env.ps1
#
# Usage (repo root, Windows PowerShell):
#   . .\tools\mainnet_rpc_env.ps1
#   powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\solana_account_state.ps1 -Owner 8s9BcTUTXmWmZVPDrkoMNKsU6n1dRsihySv1bSteSvMQ
#
# JSON only (stdout, UTF-8) for piping:
#   powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\solana_account_state.ps1 -Owner <PUBKEY> -Json
#
# Save JSON for later:
#   powershell ... -File .\tools\solana_account_state.ps1 -Owner <PUBKEY> -OutJson .\data\tmp\account_state.json

param(
    [Parameter(Mandatory = $true)]
    [string] $Owner,

    # Override RPC; default: $env:SOLANA_RPC_URL, then public mainnet beta.
    [string] $RpcUrl = "",

    [switch] $Json,

    # Write the same structured object as JSON to this path (in addition to console behavior).
    [string] $OutJson = ""
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$SPL_TOKEN = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA"
$SPL_TOKEN_2022 = "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb"

function Fail([string]$msg) { throw ("[solana-account-state] " + $msg) }
function Info([string]$msg) { if (-not $Json) { Write-Host ("[solana-account-state] " + $msg) } }

function Get-RpcUrlList {
    $primary = $RpcUrl
    if ([string]::IsNullOrWhiteSpace($primary)) {
        $primary = $env:SOLANA_RPC_URL
    }
    if ([string]::IsNullOrWhiteSpace($primary)) {
        $primary = "https://api.mainnet-beta.solana.com"
    }
    $urls = New-Object System.Collections.Generic.List[string]
    [void]$urls.Add($primary.Trim())
    $fb = $env:SOLANA_RPC_FALLBACK_URLS
    if (-not [string]::IsNullOrWhiteSpace($fb)) {
        foreach ($p in ($fb.Split(","))) {
            $t = $p.Trim()
            if ($t.Length -gt 0 -and -not $urls.Contains($t)) { [void]$urls.Add($t) }
        }
    }
    # Extra public endpoints reduce single-host 429 / 503 during read-only queries (no paid RPC required).
    foreach ($def in @(
            "https://solana.publicnode.com",
            "https://solana-api.projectserum.com"
        )) {
        if (-not $urls.Contains($def)) { [void]$urls.Add($def) }
    }
    return $urls
}

function Invoke-SolanaRpc {
    param(
        [Parameter(Mandatory = $true)]
        [string[]] $RpcUrls,
        [Parameter(Mandatory = $true)]
        [string] $BodyJson
    )
    $lastErr = $null
    foreach ($u in $RpcUrls) {
        try {
            $r = Invoke-RestMethod -Uri $u -Method Post -Body $BodyJson `
                -ContentType "application/json; charset=utf-8" -TimeoutSec 60
            $errProp = $r.PSObject.Properties["error"]
            if ($null -ne $errProp -and $null -ne $errProp.Value) {
                $lastErr = ($errProp.Value | ConvertTo-Json -Compress -Depth 6)
                continue
            }
            return @{ Ok = $true; Url = $u; Response = $r }
        } catch {
            $lastErr = $_.Exception.Message
            continue
        }
    }
    Fail ("All RPC endpoints failed. Last error: " + $lastErr)
}

function Parse-TokenRows {
    param(
        [object] $RpcResult,
        [string] $ProgramLabel
    )
    $rows = New-Object System.Collections.Generic.List[hashtable]
    if ($null -eq $RpcResult -or $null -eq $RpcResult.result -or $null -eq $RpcResult.result.value) {
        return $rows
    }
    $slot = $null
    if ($null -ne $RpcResult.result.context) {
        $slot = $RpcResult.result.context.slot
    }
    foreach ($entry in $RpcResult.result.value) {
        $ta = $entry.pubkey
        $parsed = $entry.account.data.parsed
        if ($null -eq $parsed) { continue }
        $info = $parsed.info
        if ($null -eq $info) { continue }
        $mint = [string]$info.mint
        $own = [string]$info.owner
        $amt = $info.tokenAmount
        $raw = if ($null -ne $amt.amount) { [string]$amt.amount } else { "0" }
        $dec = if ($null -ne $amt.decimals) { [int]$amt.decimals } else { 0 }
        $ui = $amt.uiAmount
        if ($null -eq $ui) {
            try { $ui = [double]$amt.uiAmountString } catch { $ui = $null }
        }
        [void]$rows.Add(@{
                token_account    = [string]$ta
                mint             = $mint
                mint_program     = $ProgramLabel
                owner            = $own
                amount_raw       = $raw
                decimals         = $dec
                ui_amount        = $ui
                ui_amount_string = if ($null -ne $amt.uiAmountString) { [string]$amt.uiAmountString } else { $null }
                slot             = $slot
            })
    }
    return $rows
}

$ownerTrim = $Owner.Trim()
if ($ownerTrim.Length -lt 32) { Fail "Owner looks invalid (too short)." }

$rpcUrls = Get-RpcUrlList
Info ("RPC try order: " + ($rpcUrls -join " | "))

$bodyBalance = (@{
        jsonrpc = "2.0"
        id      = 1
        method  = "getBalance"
        params  = @($ownerTrim)
    } | ConvertTo-Json -Compress -Depth 6)

$balWrap = Invoke-SolanaRpc -RpcUrls $rpcUrls -BodyJson $bodyBalance
$lamports = [int64]$balWrap.Response.result.value
$slotBalance = $null
if ($null -ne $balWrap.Response.result.context) { $slotBalance = $balWrap.Response.result.context.slot }

function Build-TokenBody([string]$programId) {
    # RPC expects filter object with a single key (`programId` or `mint`), and encoding in the optional third param.
    return (@{
            jsonrpc = "2.0"
            id      = 1
            method  = "getTokenAccountsByOwner"
            params  = @(
                $ownerTrim,
                @{ programId = $programId },
                @{ encoding = "jsonParsed" }
            )
        } | ConvertTo-Json -Compress -Depth 8)
}

$tok1 = Invoke-SolanaRpc -RpcUrls $rpcUrls -BodyJson (Build-TokenBody $SPL_TOKEN)
$rows1 = Parse-TokenRows -RpcResult $tok1.Response -ProgramLabel "spl-token"

$tok2 = Invoke-SolanaRpc -RpcUrls $rpcUrls -BodyJson (Build-TokenBody $SPL_TOKEN_2022)
$rows2 = Parse-TokenRows -RpcResult $tok2.Response -ProgramLabel "spl-token-2022"

$allRows = New-Object System.Collections.Generic.List[hashtable]
foreach ($r in $rows1) { [void]$allRows.Add($r) }
foreach ($r in $rows2) { [void]$allRows.Add($r) }

$slot = $slotBalance
if ($null -eq $slot) {
    foreach ($r in $allRows) {
        if ($null -ne $r.slot) { $slot = $r.slot; break }
    }
}

$out = [ordered]@{
    schema_version = 1
    fetched_at_utc = (Get-Date).ToUniversalTime().ToString("o")
    owner          = $ownerTrim
    rpc_url_used   = $balWrap.Url
    slot           = $slot
    native_sol     = @{
        lamports = $lamports
        sol      = [math]::Round([double]$lamports / 1e9, 9)
    }
    spl_token_accounts = @($allRows)
}

# Pretty JSON for on-disk export; stdout -Json must be one line so callers can parse with a single line.
$jsonPretty = ($out | ConvertTo-Json -Depth 12 -Compress:$false)

if (-not [string]::IsNullOrWhiteSpace($OutJson)) {
    $fullOut = [System.IO.Path]::GetFullPath($OutJson)
    $dir = Split-Path -Parent $fullOut
    if (-not [string]::IsNullOrWhiteSpace($dir) -and -not (Test-Path -LiteralPath $dir)) {
        New-Item -ItemType Directory -Path $dir -Force | Out-Null
    }
    [System.IO.File]::WriteAllText($fullOut, $jsonPretty, [System.Text.UTF8Encoding]::new($false))
    Info ("Wrote JSON: " + $fullOut)
}

if ($Json) {
    [Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false)
    Write-Output ($out | ConvertTo-Json -Depth 12 -Compress)
} else {
    Write-Host ("Owner:       " + $ownerTrim)
    Write-Host ("RPC:         " + $balWrap.Url)
    if ($null -ne $slot) { Write-Host ("Slot (ctx):  " + $slot) }
    Write-Host ("SOL:         " + $out.native_sol.sol + " (" + $lamports + " lamports)")
    Write-Host ""
    Write-Host "SPL token accounts:"
    if ($allRows.Count -eq 0) {
        Write-Host "  (none)"
    } else {
        foreach ($r in $allRows) {
            $line = "  " + $r.mint + "  " + $r.ui_amount + "  acct=" + $r.token_account + "  [" + $r.mint_program + "]"
            Write-Host $line
        }
    }
}
