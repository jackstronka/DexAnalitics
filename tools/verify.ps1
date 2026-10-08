<#
.SYNOPSIS
  Same checks as CI (Rust fmt/clippy/tests + web tsc/vitest), with a summary table.
  Windows counterpart of `make verify`; used by .githooks/pre-push.

.EXAMPLE
  .\tools\verify.ps1
  .\tools\verify.ps1 -SkipWeb
#>
param(
    [switch]$SkipRust,
    [switch]$SkipWeb
)

$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path -Parent $PSScriptRoot
Set-Location $repoRoot
$env:LOGLEVEL = 'WARN'

$steps = @()
if (-not $SkipRust) {
    $steps += @{ Name = 'rust: fmt';    Dir = '.';   Cmd = 'cargo'; Args = @('fmt', '--all', '--check') }
    $steps += @{ Name = 'rust: clippy'; Dir = '.';   Cmd = 'cargo'; Args = @('clippy', '--workspace', '--all-targets', '--all-features', '--', '-D', 'warnings') }
    $steps += @{ Name = 'rust: tests';  Dir = '.';   Cmd = 'cargo'; Args = @('test', '--workspace') }
}
if (-not $SkipWeb) {
    if (-not (Test-Path (Join-Path $repoRoot 'web/node_modules'))) {
        Write-Host "verify: web/node_modules missing - run 'cd web; npm install' first" -ForegroundColor Red
        exit 1
    }
    $steps += @{ Name = 'web: api.gen'; Dir = 'web'; Cmd = 'npm.cmd'; Args = @('run', 'check:api-gen') }
    $steps += @{ Name = 'web: tsc';     Dir = 'web'; Cmd = 'npx.cmd'; Args = @('tsc', '--noEmit') }
    $steps += @{ Name = 'web: vitest';  Dir = 'web'; Cmd = 'npx.cmd'; Args = @('vitest', 'run') }
}

$results = @()
$failed = $false
foreach ($s in $steps) {
    Write-Host "`n=== $($s.Name) ===" -ForegroundColor Cyan
    $sw = [Diagnostics.Stopwatch]::StartNew()
    Push-Location (Join-Path $repoRoot $s.Dir)
    try {
        & $s.Cmd @($s.Args)
        $code = $LASTEXITCODE
    } finally {
        Pop-Location
    }
    $sw.Stop()
    $status = if ($code -eq 0) { 'OK' } else { 'FAIL' }
    $results += [pscustomobject]@{ Step = $s.Name; Status = $status; Time = '{0:N0}s' -f $sw.Elapsed.TotalSeconds }
    if ($code -ne 0) { $failed = $true; break }
}

foreach ($s in ($steps | Select-Object -Skip $results.Count)) {
    $results += [pscustomobject]@{ Step = $s.Name; Status = 'SKIPPED'; Time = '' }
}

Write-Host "`n=== verify summary ===" -ForegroundColor Cyan
$results | Format-Table -AutoSize | Out-String | Write-Host
if ($failed) {
    Write-Host "verify: FAIL - fix and retry (from pre-push: 'git push --no-verify' skips once; CI still enforces)" -ForegroundColor Red
    exit 1
}
Write-Host 'verify: OK' -ForegroundColor Green
exit 0
