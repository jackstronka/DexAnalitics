# Stops the PowerShell script runner (tools/script_runner/Start-ClmmScriptRunner.ps1) if running.

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function IsRunnerCommandLine([string]$cmd) {
  if ([string]::IsNullOrWhiteSpace($cmd)) { return $false }
  return ($cmd -like "*Start-ClmmScriptRunner.ps1*")
}

$killed = 0
Get-CimInstance Win32_Process -ErrorAction SilentlyContinue | ForEach-Object {
  if (IsRunnerCommandLine $_.CommandLine) {
    try {
      Write-Host ("Stopping runner PID {0} ..." -f $_.ProcessId)
      Stop-Process -Id $_.ProcessId -Force -ErrorAction Stop
      $killed++
    } catch {
      Write-Warning ("Could not stop PID {0}: {1}" -f $_.ProcessId, $_)
    }
  }
}

if ($killed -eq 0) {
  Write-Host "No running CLMM script runner process found."
} else {
  Write-Host ("Stopped {0} runner process(es)." -f $killed)
}
