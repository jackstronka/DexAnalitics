# Stops the 10-minute snapshot loop (run-snapshot-loop.ps1) if running, then starts it again
# with pinned RPC (same defaults as scripts/windows/run-snapshot-loop.ps1).
# Does NOT touch the 5m loop (run-snapshot-loop-5m.ps1).
#
# Usage (from repo root):
#   pwsh -File .\tools\restart_snapshot_loop_10m.ps1
# Optional:
#   pwsh -File .\tools\restart_snapshot_loop_10m.ps1 -Configuration release -IntervalMinutes 10

param(
    [int]$IntervalMinutes = 10,
    [string]$Configuration = "release",
    [string]$RpcPrimary = "https://api.mainnet-beta.solana.com",
    [string]$RpcFallbacks = "https://solana-api.projectserum.com,https://rpc.ankr.com/solana"
)

$ErrorActionPreference = "Stop"
$RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$LoopScript = Join-Path $RepoRoot "scripts\windows\run-snapshot-loop.ps1"

if (-not (Test-Path $LoopScript)) {
    Write-Error "Missing: $LoopScript"
}

function Test-Is10mLoopCommandLine([string]$cmd) {
    if ([string]::IsNullOrWhiteSpace($cmd)) { return $false }
    if ($cmd -notlike '*run-snapshot-loop.ps1*') { return $false }
    if ($cmd -like '*run-snapshot-loop-5m.ps1*') { return $false }
    return $true
}

$stopped = @()
Get-CimInstance Win32_Process -ErrorAction SilentlyContinue | ForEach-Object {
    if (Test-Is10mLoopCommandLine $_.CommandLine) {
        $stopped += [pscustomobject]@{ Pid = $_.ProcessId; Line = $_.CommandLine }
    }
}

if ($stopped.Count -eq 0) {
    Write-Host "No running 10m snapshot-loop PowerShell process found (nothing to stop)."
} else {
    foreach ($p in $stopped) {
        Write-Host "Stopping PID $($p.Pid) ..."
        try {
            Stop-Process -Id $p.Pid -Force -ErrorAction Stop
        } catch {
            Write-Warning "Could not stop PID $($p.Pid): $_"
        }
    }
    Start-Sleep -Seconds 2
}

$pwsh = Get-Command pwsh -ErrorAction SilentlyContinue
$exe = if ($pwsh) { $pwsh.Source } else { "powershell.exe" }

$argList = @(
    "-NoProfile",
    "-ExecutionPolicy", "Bypass",
    "-File", $LoopScript,
    "-IntervalMinutes", "$IntervalMinutes",
    "-Configuration", $Configuration,
    "-RpcPrimary", $RpcPrimary,
    "-RpcFallbacks", $RpcFallbacks
)

Write-Host "Starting 10m loop: $exe $($argList -join ' ')"
Start-Process -FilePath $exe -ArgumentList $argList -WorkingDirectory $RepoRoot -WindowStyle Minimized

Write-Host "Done. Log: $RepoRoot\data\snapshot_logs\snapshot-loop.log"
