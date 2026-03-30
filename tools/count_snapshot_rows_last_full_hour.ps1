# Count snapshot rows in the last full UTC hour for given Orca pools/files.
param(
    [string] $RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path,
    # Optional: count a specific local hour window, e.g. -LocalStartHour 14 -LocalEndHour 15
    [int] $LocalStartHour = -1,
    [int] $LocalEndHour = -1
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$nowLocal = Get-Date
$startUtc = $null
$endUtc = $null
if ($LocalStartHour -ge 0 -and $LocalEndHour -ge 0) {
    $startLocal = $nowLocal.Date.AddHours($LocalStartHour)
    $endLocal = $nowLocal.Date.AddHours($LocalEndHour)
    $startUtc = $startLocal.ToUniversalTime()
    $endUtc = $endLocal.ToUniversalTime()
    Write-Host ("Local window: {0}..{1}" -f $startLocal.ToString("o"), $endLocal.ToString("o"))
    Write-Host ("UTC window:   {0}..{1}" -f $startUtc.ToString("o"), $endUtc.ToString("o"))
} else {
    $endUtc = [datetime]::UtcNow
    $endUtc = [datetime]::SpecifyKind($endUtc.Date.AddHours($endUtc.Hour), [System.DateTimeKind]::Utc)
    $startUtc = $endUtc.AddHours(-1)
    Write-Host ("Window UTC: {0}..{1}" -f $startUtc.ToString("o"), $endUtc.ToString("o"))
}

$hourPrefix = $startUtc.ToString("yyyy-MM-ddTHH:")

Write-Host ("Hour prefix match: {0}" -f $hourPrefix)

$targets = @(
    @{ pool = "Czfq3xZZDmsdGdUyrNLtRhGc47cXcZtLG4crryfu44zE"; file = "snapshots.jsonl" },
    @{ pool = "Czfq3xZZDmsdGdUyrNLtRhGc47cXcZtLG4crryfu44zE"; file = "snapshots_5m.jsonl" },
    @{ pool = "HktfL7iwGKT5QHjywQkcDnZXScoh811k7akrMZJkCcEF"; file = "snapshots.jsonl" },
    @{ pool = "HktfL7iwGKT5QHjywQkcDnZXScoh811k7akrMZJkCcEF"; file = "snapshots_5m.jsonl" },
    @{ pool = "HxA6SKW5qA4o12fjVgTpXdq2YnZ5Zv1s7SB4FFomsyLM"; file = "snapshots.jsonl" },
    @{ pool = "HxA6SKW5qA4o12fjVgTpXdq2YnZ5Zv1s7SB4FFomsyLM"; file = "snapshots_5m.jsonl" }
)

foreach ($t in $targets) {
    $path = Join-Path (Join-Path (Join-Path $RepoRoot "data/pool-snapshots/orca") $t.pool) $t.file
    if (!(Test-Path $path)) {
        Write-Host ("{0} {1} => MISSING ({2})" -f $t.pool, $t.file, $path)
        continue
    }
    $pattern = ("`"ts_utc`":`"" + $hourPrefix)
    $cnt = (Select-String -Path $path -SimpleMatch -Pattern $pattern | Measure-Object).Count
    Write-Host ("{0} {1} => rows_in_window~={2}" -f $t.pool, $t.file, $cnt)
}

