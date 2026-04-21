<#
.SYNOPSIS
  Rejestruje zadanie Harmonogramu zadań Windows: co N minut uruchamia tools/snapshot_health_alert.ps1 (Slack przy NOT OK).

.DESCRIPTION
  Jednorazowa konfiguracja — potem alerty działają bez ręcznego odpalania skryptów.
  Wymaga `.env` w rootcie repo z `SLACK_WEBHOOK_URL` (patrz doc/OPERATIONAL_CONTINUITY.md), chyba że używasz -SkipSlack.

  Alternatywa bez Harmonogramu: długa pętla tools/data_alerts_loop.ps1 pod Shawl/NSSM.

  Uwaga: skrypty pod `scripts/windows/` mogą być u Ciebie w `.gitignore` — ten plik jest w `tools/`, żeby był wersjonowany w gicie.

.PARAMETER IntervalMinutes
  Co ile minut uruchamiać snapshot_health_alert (domyślnie 5).

.EXAMPLE
  powershell -NoProfile -ExecutionPolicy Bypass -File tools\register_snapshot_health_scheduled_task.ps1

.EXAMPLE
  powershell -NoProfile -ExecutionPolicy Bypass -File tools\register_snapshot_health_scheduled_task.ps1 -IntervalMinutes 5
#>
[CmdletBinding()]
param(
    [string] $RepoRoot = "",
    [string] $TaskName = "CLMM-SnapshotHealthAlert",
    [int] $IntervalMinutes = 5,
    [switch] $SkipSlack
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

if (-not $RepoRoot) {
    $scriptDir = $null
    if ($PSScriptRoot) {
        $scriptDir = $PSScriptRoot
    } elseif ($MyInvocation.MyCommand.Path) {
        $scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
    }

    if ($scriptDir) {
        $RepoRoot = (Resolve-Path (Join-Path $scriptDir "..")).Path
    } else {
        $RepoRoot = (Get-Location).Path
    }
}

if ($IntervalMinutes -lt 1) {
    throw "IntervalMinutes must be >= 1"
}

$toolScript = Join-Path $RepoRoot "tools\snapshot_health_alert.ps1"
if (-not (Test-Path -LiteralPath $toolScript)) {
    throw "Missing $toolScript"
}

$slackArg = if ($SkipSlack) { " -SkipSlack" } else { "" }
$psArgs = "-NoProfile -ExecutionPolicy Bypass -File `"$toolScript`"$slackArg"

$powershellExe = Join-Path $env:SystemRoot "System32\WindowsPowerShell\v1.0\powershell.exe"
if (-not (Test-Path -LiteralPath $powershellExe)) {
    throw "powershell.exe not found at $powershellExe"
}

$action = New-ScheduledTaskAction -Execute $powershellExe -Argument $psArgs -WorkingDirectory $RepoRoot

# Powtarzaj co IntervalMinutes (praktycznie „w nieskończoność” w UI Harmonogramu).
$repeatDuration = New-TimeSpan -Days 9999
$startAnchor = (Get-Date).AddMinutes(1)
$triggerRepeat = New-ScheduledTaskTrigger -Once -At $startAnchor `
    -RepetitionInterval (New-TimeSpan -Minutes $IntervalMinutes) `
    -RepetitionDuration $repeatDuration

# Po zalogowaniu użytkownika (to samo konto, które rejestruje zadanie).
$triggerLogon = New-ScheduledTaskTrigger -AtLogOn

$settings = New-ScheduledTaskSettingsSet `
    -AllowStartIfOnBatteries `
    -DontStopIfGoingOnBatteries `
    -StartWhenAvailable `
    -ExecutionTimeLimit (New-TimeSpan -Hours 1) `
    -MultipleInstances IgnoreNew

$principal = New-ScheduledTaskPrincipal -UserId $env:USERNAME -LogonType Interactive -RunLevel Limited

Unregister-ScheduledTask -TaskName $TaskName -Confirm:$false -ErrorAction SilentlyContinue

Register-ScheduledTask `
    -TaskName $TaskName `
    -Description "CLMM: automatyczny monitoring kolektorów snapshotów (snapshot_health_alert). Wymaga działających pętli run-snapshot-loop*.ps1 i .env ze SLACK_WEBHOOK_URL." `
    -Action $action `
    -Trigger @($triggerLogon, $triggerRepeat) `
    -Settings $settings `
    -Principal $principal `
    -Force | Out-Null

Write-Host "OK: zarejestrowano zadanie '$TaskName' (co $IntervalMinutes min, working dir: $RepoRoot)."
Write-Host "Test: Start-ScheduledTask -TaskName '$TaskName'"
Write-Host "Logi skryptu: data/snapshot_logs/ oraz ewentualny Slack przy NOT OK."
