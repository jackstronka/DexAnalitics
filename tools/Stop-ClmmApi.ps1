#Requires -Version 5.1
<#
.SYNOPSIS
  Zatrzymuje wszystkie procesy API uruchomione przez lokalne skrypty CLMM (exe/cargo/pwsh wrapper).
#>

Set-StrictMode -Version Latest
$ErrorActionPreference = "Continue"

$killed = New-Object System.Collections.Generic.HashSet[int]

function Stop-ByPid {
  param([int]$ProcessId)
  if ($ProcessId -le 0) { return }
  if ($killed.Contains($ProcessId)) { return }
  try {
    Stop-Process -Id $ProcessId -Force -ErrorAction Stop
    [void]$killed.Add($ProcessId)
  } catch {
    # best effort
  }
}

# 1) Direct binary process.
Get-Process -Name 'clmm-lp-api' -ErrorAction SilentlyContinue | ForEach-Object {
  Stop-ByPid -ProcessId $_.Id
}

# 2) Any process currently listening on API port 8081.
try {
  Get-NetTCPConnection -LocalPort 8081 -State Listen -ErrorAction SilentlyContinue | ForEach-Object {
    Stop-ByPid -ProcessId $_.OwningProcess
  }
} catch {
  # best effort
}

# 3) pwsh/cargo wrappers launched by Start-ClmmApi-8081.
try {
  $candidates = Get-CimInstance Win32_Process -ErrorAction SilentlyContinue |
    Where-Object {
      $name = [string]$_.Name
      $cmd = [string]$_.CommandLine
      if ([string]::IsNullOrWhiteSpace($cmd)) { return $false }
      if ($name -ieq 'pwsh.exe' -and ($cmd -match 'Start-ClmmApi-8081\.ps1' -or $cmd -match 'cargo run -q -p clmm-lp-api --bin clmm-lp-api')) { return $true }
      if ($name -ieq 'cargo.exe' -and $cmd -match '-p clmm-lp-api' -and $cmd -match '--bin clmm-lp-api') { return $true }
      return $false
    }
  foreach ($p in $candidates) {
    Stop-ByPid -ProcessId ([int]$p.ProcessId)
  }
} catch {
  # best effort
}

if ($killed.Count -eq 0) {
  Write-Host 'Brak aktywnych procesów API do zatrzymania.' -ForegroundColor DarkGray
  exit 0
}

Write-Host ("Zatrzymano procesy API (PID): " + (($killed | Sort-Object) -join ', ')) -ForegroundColor Yellow
Write-Host 'Gotowe.' -ForegroundColor Green
