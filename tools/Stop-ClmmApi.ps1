#Requires -Version 5.1
<#
.SYNOPSIS
  Zatrzymuje proces(y) clmm-lp-api.exe (Windows blokuje nadpisanie pliku przy cargo run, gdy API nadal działa).
#>
$procs = Get-Process -Name 'clmm-lp-api' -ErrorAction SilentlyContinue
if (-not $procs) {
    Write-Host 'Brak uruchomionego clmm-lp-api.' -ForegroundColor DarkGray
    exit 0
}
Write-Host "Zatrzymuję clmm-lp-api (PID: $($procs.Id -join ', '))..." -ForegroundColor Yellow
$procs | Stop-Process -Force
Write-Host 'Gotowe.' -ForegroundColor Green
