<#
.SYNOPSIS
  R2: emit per-area test report from JUnit XML (nextest + vitest).
#>
param(
    [Parameter(Mandatory = $true)]
    [string[]]$Junit,
    [string]$OutDir = 'target/test-report'
)
$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path -Parent $PSScriptRoot
Set-Location $repoRoot
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
$args = @()
foreach ($f in $Junit) {
    if (Test-Path $f) {
        $args += @('--junit', $f)
    } else {
        Write-Host "test-report: skip missing $f" -ForegroundColor Yellow
    }
}
if ($args.Count -eq 0) {
    Write-Host 'test-report: no junit files' -ForegroundColor Red
    exit 1
}
# Do not fail verify solely on report exit (tests already gated the run).
& py -3 tools/test_report.py @args --out-dir $OutDir
Get-Content (Join-Path $OutDir 'report.md') -ErrorAction SilentlyContinue | Write-Host
exit 0
