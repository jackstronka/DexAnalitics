#Requires -Version 5.1
<#
.SYNOPSIS
  Uruchamia dashboard w jednym oknie: zatrzymuje stare procesy, potem API (:8080) + Vite (:3000) z połączonymi logami.

.DESCRIPTION
  Wywołuje `npm run dev:stack` w katalogu web/ (Node: concurrently + kill-port).
  Ustawia CLMM_REPO_ROOT wewnątrz skryptu Node — nie trzeba nic ustawiać ręcznie.

  Uruchom: Start-Dashboard.bat w korzeniu repo albo ten plik z PowerShell.
#>

$ErrorActionPreference = 'Stop'
$RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$WebDir = Join-Path $RepoRoot 'web'

if (-not (Test-Path (Join-Path $WebDir 'package.json'))) {
    Write-Error "Nie znaleziono web/package.json — uruchom z repozytorium CLMM-Liquidity-Provider."
}

if (-not (Test-Path (Join-Path $WebDir 'node_modules'))) {
    Write-Host '[Start-Dashboard] npm install w web/ (pierwszy raz)...' -ForegroundColor Cyan
    Push-Location $WebDir
    try {
        npm install
    } finally {
        Pop-Location
    }
}

Set-Location $WebDir
npm start
