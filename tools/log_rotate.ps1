#Requires -Version 5.1
<#
.SYNOPSIS
  Delete old log/report artifacts under data/ (simple retention).

.DESCRIPTION
  Intended to be run as a scheduled one-shot (Task Scheduler) once per day.
  Keeps the repo from growing unbounded when running long-lived loops under Shawl/NSSM.

  Default targets:
  - data/logs/*.log
  - data/snapshot_logs/*.log
  - data/reports/*.json
  - data/agent-alerts/** (state/history)

.PARAMETER RepoRoot
  Repository root. Default: parent of tools/

.PARAMETER KeepDays
  Delete files older than this many days. Default: 7.

.EXAMPLE
  powershell -NoProfile -ExecutionPolicy Bypass -File tools/log_rotate.ps1 -KeepDays 14
#>
[CmdletBinding()]
param(
  [string] $RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path,
  [int] $KeepDays = 7,
  [switch] $WhatIf
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

Set-Location $RepoRoot

$cutoff = (Get-Date).ToUniversalTime().AddDays(-1 * $KeepDays)

function Prune([string] $path, [string[]] $include) {
  if (-not (Test-Path -LiteralPath $path)) { return }
  Get-ChildItem -Path $path -Recurse -File -Include $include -ErrorAction SilentlyContinue |
    Where-Object { $_.LastWriteTimeUtc -lt $cutoff } |
    ForEach-Object {
      if ($WhatIf) {
        Write-Host ("[log-rotate] would delete: {0}" -f $_.FullName)
      } else {
        try {
          Remove-Item -LiteralPath $_.FullName -Force -ErrorAction Stop
          Write-Host ("[log-rotate] deleted: {0}" -f $_.FullName)
        } catch {
          Write-Warning ("[log-rotate] failed delete: {0} err={1}" -f $_.FullName, $_)
        }
      }
    }
}

Write-Host ("[log-rotate] RepoRoot={0} KeepDays={1} cutoff_utc={2}" -f $RepoRoot, $KeepDays, $cutoff.ToString("o"))

Prune -path (Join-Path $RepoRoot "data\\logs") -include @("*.log")
Prune -path (Join-Path $RepoRoot "data\\snapshot_logs") -include @("*.log")
Prune -path (Join-Path $RepoRoot "data\\reports") -include @("*.json", "*.csv")
Prune -path (Join-Path $RepoRoot "data\\agent-alerts") -include @("*.json", "*.jsonl")

Write-Host "[log-rotate] done"

