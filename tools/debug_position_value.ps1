param(
  [Parameter(Mandatory = $true)]
  [string]$Position,
  [string]$BaseUrl = "http://127.0.0.1:8080/api/v1",
  [string]$ApiKey = ""
)

$ErrorActionPreference = "Stop"

$headers = @{}
if ($ApiKey.Trim().Length -gt 0) {
  $headers["X-API-Key"] = $ApiKey.Trim()
}

$healthUrl = "$BaseUrl/health"
Write-Host $healthUrl
try {
  $r = Invoke-WebRequest -Method GET -Uri $healthUrl -TimeoutSec 10 -Headers $headers
  Write-Host ("STATUS " + $r.StatusCode)
  Write-Host $r.Content
} catch {
  $resp = $_.Exception.Response
  if ($null -ne $resp) {
    Write-Host ("ERR HTTP " + [int]$resp.StatusCode + " " + $resp.StatusDescription)
  } else {
    Write-Host ("ERR " + $_.Exception.Message)
  }
}
Write-Host "---"

$paths = @(
  "/positions/$Position",
  "/positions/$Position/lifecycle-summary",
  "/positions/$Position/stream-pnl",
  "/positions/$Position/stream-performance",
  "/positions/$Position/diagnostics"
)

foreach ($p in $paths) {
  $url = "$BaseUrl$p"
  Write-Host $url
  try {
    $r = Invoke-RestMethod -Method GET -Uri $url -TimeoutSec 10 -Headers $headers
    $json = $r | ConvertTo-Json -Depth 20
    if ($json.Length -gt 2500) { $json = $json.Substring(0, 2500) + " ..." }
    Write-Host $json
  } catch {
    $resp = $_.Exception.Response
    if ($null -ne $resp) {
      Write-Host ("ERR HTTP " + [int]$resp.StatusCode + " " + $resp.StatusDescription)
    } else {
      Write-Host ("ERR " + $_.Exception.Message)
    }
  }
  Write-Host "---"
}

