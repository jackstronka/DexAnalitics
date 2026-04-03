#Requires -Version 7.2
<#
.SYNOPSIS
  Localhost HTTP runner for allowlisted tools/*.ps1 — see tools/script_runner/README.md
#>
Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Import-DotEnvIfPresent {
  param([Parameter(Mandatory)][string]$RepoRoot)

  $envPath = Join-Path $RepoRoot ".env"
  if (-not (Test-Path -LiteralPath $envPath)) {
    return $false
  }

  foreach ($line in (Get-Content -LiteralPath $envPath)) {
    $t = $line.Trim()
    if ($t -eq "" -or $t.StartsWith("#")) { continue }
    $m = [regex]::Match($t, '^\s*([A-Za-z_][A-Za-z0-9_]*)\s*=\s*(.*)\s*$')
    if (-not $m.Success) { continue }

    $k = $m.Groups[1].Value
    $v = $m.Groups[2].Value

    # Strip surrounding quotes
    if ($v.Length -ge 2) {
      if (($v.StartsWith('"') -and $v.EndsWith('"')) -or ($v.StartsWith("'") -and $v.EndsWith("'"))) {
        $v = $v.Substring(1, $v.Length - 2)
      }
    }

    # Do not override explicitly set env vars
    $existing = [Environment]::GetEnvironmentVariable($k, "Process")
    if ([string]::IsNullOrWhiteSpace($existing)) {
      [Environment]::SetEnvironmentVariable($k, $v, "Process")
    }
  }

  return $true
}

function Get-RepoRoot {
  if ($env:CLMM_REPO_ROOT -and $env:CLMM_REPO_ROOT.Trim().Length -gt 0) {
    return (Resolve-Path -LiteralPath $env:CLMM_REPO_ROOT.Trim()).Path
  }
  return (Get-Location).Path
}

function Read-Manifest {
  param([string]$RepoRoot)
  $p = Join-Path $RepoRoot "tools\scripts-manifest.json"
  if (-not (Test-Path -LiteralPath $p)) { return $null }
  return (Get-Content -LiteralPath $p -Raw | ConvertFrom-Json)
}

function Test-ScriptAllowed {
  param(
    [object]$Manifest,
    [string]$ScriptId
  )
  foreach ($s in $Manifest.scripts) {
    if ($s.id -eq $ScriptId) { return $s }
  }
  return $null
}

# Zgodnie z API: najpierw manifest, potem `tools/{id}.ps1` (tylko top-level tools).
function Resolve-ScriptEntry {
  param(
    [string]$RepoRoot,
    [object]$Manifest,
    [string]$ScriptId
  )
  if ($null -ne $Manifest -and $Manifest.scripts) {
    $m = Test-ScriptAllowed -Manifest $Manifest -ScriptId $ScriptId
    if ($null -ne $m) { return $m }
  }
  $rel = "tools/$ScriptId.ps1"
  $ps1 = Join-Path $RepoRoot ($rel -replace "/", "\")
  if (Test-Path -LiteralPath $ps1) {
    return [pscustomobject]@{
      id       = $ScriptId
      path     = $rel
      runnable = $true
    }
  }
  return $null
}

function Join-UnderRepo {
  param([string]$RepoRoot, [string]$RelativePath)
  $full = [System.IO.Path]::GetFullPath((Join-Path $RepoRoot $RelativePath))
  $tools = [System.IO.Path]::GetFullPath((Join-Path $RepoRoot "tools"))
  if (-not $full.StartsWith($tools, [System.StringComparison]::OrdinalIgnoreCase)) {
    throw "path escapes tools directory: $RelativePath"
  }
  if (-not ($full -like "*.ps1")) { throw "not a .ps1 file" }
  return $full
}

function Limit-Str {
  param([string]$s, [int]$Max = 4000)
  if ([string]::IsNullOrEmpty($s)) { return "" }
  if ($s.Length -le $Max) { return $s }
  return $s.Substring(0, $Max) + "…"
}

function Append-RunJsonl {
  param(
    [string]$RepoRoot,
    [hashtable]$Record
  )
  $dir = Join-Path $RepoRoot "data"
  if (-not (Test-Path -LiteralPath $dir)) {
    New-Item -ItemType Directory -Path $dir | Out-Null
  }
  $path = Join-Path $dir "script_runs.jsonl"
  $line = ($Record | ConvertTo-Json -Compress -Depth 6)
  Add-Content -LiteralPath $path -Value $line -Encoding utf8
}

function Invoke-AllowedScript {
  param(
    [string]$RepoRoot,
    [string]$Ps1Path,
    [string]$Trigger
  )
  $sw = [System.Diagnostics.Stopwatch]::StartNew()
  $stdout = Join-Path ([System.IO.Path]::GetTempPath()) ("clmm-out-" + [Guid]::NewGuid().ToString("n") + ".txt")
  $stderr = Join-Path ([System.IO.Path]::GetTempPath()) ("clmm-err-" + [Guid]::NewGuid().ToString("n") + ".txt")
  try {
    $p = Start-Process -FilePath "pwsh" `
      -ArgumentList @("-NoProfile", "-File", $Ps1Path) `
      -WorkingDirectory $RepoRoot `
      -PassThru `
      -NoNewWindow `
      -RedirectStandardOutput $stdout `
      -RedirectStandardError $stderr
    $timeoutSec = 30 * 60
    $null = Wait-Process -Id $p.Id -Timeout $timeoutSec -ErrorAction SilentlyContinue
    if (-not $p.HasExited) {
      try { Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue } catch {}
      throw "script timed out after ${timeoutSec}s"
    }
    $code = $p.ExitCode
    $outT = if (Test-Path -LiteralPath $stdout) { Get-Content -LiteralPath $stdout -Raw } else { "" }
    $errT = if (Test-Path -LiteralPath $stderr) { Get-Content -LiteralPath $stderr -Raw } else { "" }
    if ($null -eq $outT) { $outT = "" }
    if ($null -eq $errT) { $errT = "" }
    $ok = ($code -eq 0)
    $errEx = if (-not $ok) { Limit-Str ($errT + "`n" + $outT) } else { $null }
    return @{
      ok = $ok
      exit_code = $code
      duration_ms = [int64]$sw.ElapsedMilliseconds
      stdout_excerpt = (Limit-Str $outT)
      stderr_excerpt = (Limit-Str $errT)
      error_excerpt = $errEx
    }
  }
  finally {
    Remove-Item -LiteralPath $stdout -ErrorAction SilentlyContinue
    Remove-Item -LiteralPath $stderr -ErrorAction SilentlyContinue
  }
}

function Send-JsonResponse {
  param(
    [System.Net.HttpListenerResponse]$Response,
    [int]$Status,
    [object]$Body
  )
  $Response.StatusCode = $Status
  $Response.ContentType = "application/json; charset=utf-8"
  $json = ($Body | ConvertTo-Json -Compress -Depth 8)
  $bytes = [System.Text.Encoding]::UTF8.GetBytes($json)
  $Response.ContentLength64 = $bytes.Length
  $Response.OutputStream.Write($bytes, 0, $bytes.Length)
  $Response.Close()
}

$repoRoot = Get-RepoRoot
$imported = Import-DotEnvIfPresent -RepoRoot $repoRoot
if ($imported) {
  Write-Host "Loaded .env from repo root (only for missing env vars)."
}

# Re-read token after .env import (allows CLMM_SCRIPT_RUNNER_TOKEN in .env).
$token = $env:CLMM_SCRIPT_RUNNER_TOKEN
if (-not $token -or $token.Trim().Length -eq 0) {
  throw "Set CLMM_SCRIPT_RUNNER_TOKEN to a shared secret (or add CLMM_SCRIPT_RUNNER_TOKEN=... in repo-root .env)."
}

$mfPath = Join-Path $repoRoot "tools\scripts-manifest.json"
if (-not (Test-Path -LiteralPath $mfPath)) {
  Write-Warning "tools/scripts-manifest.json not found — POST /run still resolves tools/{id}.ps1 on disk."
}

$port = 9847
if ($env:CLMM_SCRIPT_RUNNER_PORT -and $env:CLMM_SCRIPT_RUNNER_PORT -match '^\d+$') {
  $port = [int]$env:CLMM_SCRIPT_RUNNER_PORT
}

$prefix = "http://127.0.0.1:$port/"
$listener = New-Object System.Net.HttpListener
$listener.Prefixes.Add($prefix)
try {
  $listener.Start()
} catch {
  $err = $_.Exception.Message
  $altPort = if ($port -eq 9847) { 9857 } else { 9847 }
  $altPrefix = "http://127.0.0.1:$altPort/"
  Write-Warning "Failed to listen on $prefix ($err). Retrying on $altPrefix ..."

  $port = $altPort
  $prefix = $altPrefix
  $listener = New-Object System.Net.HttpListener
  $listener.Prefixes.Add($prefix)
  $listener.Start()
}
Write-Host "CLMM script runner listening on $prefix (repo: $repoRoot)"

while ($listener.IsListening) {
  $ctx = $listener.GetContext()
  try {
    $req = $ctx.Request
    $res = $ctx.Response
    $path = $req.Url.AbsolutePath.TrimEnd("/")
    if ($req.HttpMethod -eq "GET" -and ($path -eq "" -or $path -eq "/health")) {
      Send-JsonResponse -Response $res -Status 200 -Body @{ ok = $true; repo_root = $repoRoot }
      continue
    }
    if ($req.HttpMethod -ne "POST" -or $path -ne "/run") {
      Send-JsonResponse -Response $res -Status 404 -Body @{ error = "not found" }
      continue
    }
    $bodyRaw = ""
    $body = $null
    if ($null -ne $req.InputStream) {
      $reader = [System.IO.StreamReader]::new($req.InputStream, [System.Text.Encoding]::UTF8, $true)
      $bodyRaw = $reader.ReadToEnd()
      if (-not [string]::IsNullOrWhiteSpace($bodyRaw)) {
        $body = $bodyRaw | ConvertFrom-Json
      }
    }

    # Also accept parameters in query string for maximum compatibility (some clients may surface InputStream as null).
    $qs = $req.Url.Query
    $q = [System.Web.HttpUtility]::ParseQueryString($qs)
    $qScriptId = [string]$q.Get("script_id")
    $qTriggeredBy = [string]$q.Get("triggered_by")
    $qToken = [string]$q.Get("token")

    # Auth: prefer Authorization header, but accept token in JSON body as a fallback.
    # HttpListener can occasionally surface Headers as null depending on client/stack.
    $expected = "Bearer " + $token
    $auth = $null
    if ($null -ne $req -and $null -ne $req.Headers) {
      try { $auth = $req.Headers.Get("Authorization") } catch { $auth = $null }
    }
    $bodyToken = $null
    if ($body -and ($body.PSObject.Properties.Name -contains "token")) { $bodyToken = [string]$body.token }
    $okAuth = ($auth -eq $expected) -or ($bodyToken -eq $token) -or ($qToken -eq $token)
    if (-not $okAuth) {
      Send-JsonResponse -Response $res -Status 401 -Body @{ error = "unauthorized" }
      continue
    }

    $scriptId = $null
    if ($body -and ($body.PSObject.Properties.Name -contains "script_id")) { $scriptId = [string]$body.script_id }
    if ([string]::IsNullOrWhiteSpace($scriptId)) { $scriptId = $qScriptId }
    if ([string]::IsNullOrWhiteSpace($scriptId)) {
      Send-JsonResponse -Response $res -Status 400 -Body @{ error = "script_id required" }
      continue
    }
    $manifest = Read-Manifest -RepoRoot $repoRoot
    $entry = Resolve-ScriptEntry -RepoRoot $repoRoot -Manifest $manifest -ScriptId $scriptId
    if (-not $entry) {
      Send-JsonResponse -Response $res -Status 404 -Body @{ error = "unknown script_id" }
      continue
    }
    if ($entry.runnable -eq $false) {
      Send-JsonResponse -Response $res -Status 400 -Body @{ error = "script is not runnable (helper/example)" }
      continue
    }
    $rel = [string]$entry.path
    $ps1 = Join-UnderRepo -RepoRoot $repoRoot -RelativePath ($rel -replace "/", "\")
    if (-not (Test-Path -LiteralPath $ps1)) {
      Send-JsonResponse -Response $res -Status 400 -Body @{ error = "file missing on disk: $rel" }
      continue
    }
    $trigger = "runner_api"
    if ($body -and ($body.PSObject.Properties.Name -contains "triggered_by") -and $body.triggered_by) {
      $trigger = [string]$body.triggered_by
    } elseif (-not [string]::IsNullOrWhiteSpace($qTriggeredBy)) {
      $trigger = $qTriggeredBy
    }
    $run = Invoke-AllowedScript -RepoRoot $repoRoot -Ps1Path $ps1 -Trigger $trigger
    $ts = [DateTimeOffset]::UtcNow.ToString("o")
    $record = @{
      schema_version = 1
      script_id      = $scriptId
      ts_utc         = $ts
      ok             = $run.ok
      exit_code      = $run.exit_code
      duration_ms    = $run.duration_ms
      stdout_excerpt = $run.stdout_excerpt
      stderr_excerpt = $run.stderr_excerpt
      error_excerpt  = $run.error_excerpt
      triggered_by   = $trigger
    }
    Append-RunJsonl -RepoRoot $repoRoot -Record $record
    Send-JsonResponse -Response $res -Status 200 -Body $record
  }
  catch {
    $details = $_ | Out-String
    if (-not $details -or [string]::IsNullOrWhiteSpace($details)) {
      $details = $_.Exception.Message
    }
    Send-JsonResponse -Response $ctx.Response -Status 500 -Body @{ error = $details }
  }
}
