# Generates a new Solana keypair file for the bot at a chosen path.
# Prints ONLY the resulting pubkey + file location (does not print seed phrase to the console).
#
# Usage (Windows PowerShell):
#   powershell -NoProfile -ExecutionPolicy Bypass -File tools/new_bot_keypair.ps1
#
# Then fund the printed pubkey from Phantom.

param(
    # Where to write the keypair file (JSON array).
    [string] $OutFile = "",
    # If true, overwrite existing file.
    [switch] $Force,
    # If true, try using `wsl.exe solana-keygen` when `solana-keygen` is not in Windows PATH.
    [switch] $AllowWslFallback = $true,
    # Optional: WSL distribution name (see `wsl.exe -l -v`). If empty, default distro is used.
    [string] $WslDistro = ""
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Fail([string]$msg) { throw ("[new-bot-keypair] " + $msg) }
function Info([string]$msg) { Write-Host ("[new-bot-keypair] " + $msg) }

# Resolve default path under USERPROFILE (Windows-friendly, outside repo).
if ([string]::IsNullOrWhiteSpace($OutFile)) {
    $base = Join-Path $env:USERPROFILE ".config\solana"
    $OutFile = Join-Path $base "clmm_lp_bot_mainnet.json"
}

$outPath = [System.IO.Path]::GetFullPath($OutFile)
$outDir = Split-Path -Parent $outPath

function Has-Command([string]$name) {
    return $null -ne (Get-Command $name -ErrorAction SilentlyContinue)
}

function Convert-WindowsPathToWsl([string] $winPath) {
    if ([string]::IsNullOrWhiteSpace($winPath)) {
        Fail "Convert-WindowsPathToWsl: empty path"
    }
    $p = [System.IO.Path]::GetFullPath($winPath)
    # Expect "C:\..." style.
    if ($p.Length -lt 3 -or $p[1] -ne ':' -or ($p[2] -ne '\' -and $p[2] -ne '/')) {
        Fail ("Convert-WindowsPathToWsl: unsupported path format: " + $p)
    }
    $drive = $p.Substring(0, 1).ToLowerInvariant()
    $rest = $p.Substring(2) # keep leading "\" or "/"
    $rest = $rest -replace '\\', '/'
    if (-not $rest.StartsWith('/')) { $rest = '/' + $rest }
    return ("/mnt/" + $drive + $rest)
}

$useWsl = $false
if (-not (Has-Command "solana-keygen")) {
    if ($AllowWslFallback -and (Has-Command "wsl.exe")) {
        $useWsl = $true
        Info "solana-keygen not found in Windows PATH; falling back to WSL (wsl.exe solana-keygen)."
    } else {
        Fail "solana-keygen not found in PATH. Install Solana CLI on Windows or run with -AllowWslFallback (requires WSL + solana-keygen in WSL)."
    }
}

New-Item -ItemType Directory -Force -Path $outDir | Out-Null

if ((Test-Path $outPath) -and (-not $Force)) {
    Fail "Refusing to overwrite existing file: $outPath (re-run with -Force to overwrite)"
}

# `solana-keygen new` prints the seed phrase to stdout by default. We must NOT echo it to the console.
# Capture the output to a temp file, then delete it.
$tmp = Join-Path ([System.IO.Path]::GetTempPath()) ("clmm_lp_keygen_" + [Guid]::NewGuid().ToString("N") + ".txt")
try {
    if (-not $useWsl) {
        $args = @("new", "--no-bip39-passphrase", "--outfile", $outPath)
        if ($Force) { $args += @("--force") }
        & solana-keygen @args 2>&1 | Out-File -FilePath $tmp -Encoding utf8
    } else {
        # Convert Windows path to WSL path so the keypair is written into the Windows filesystem.
        # Avoid relying on `wslpath` because argument passing can be brittle across shells.
        $wslPath = Convert-WindowsPathToWsl $outPath
        $forceArg = if ($Force) { " --force" } else { "" }
        # Some terminals (especially when called from nested shells) can cause bash to emit:
        #   "your 131072x1 screen size is bogus. expect trouble"
        # It is harmless but may be treated as an error record in PowerShell; we suppress it.
        $cmd = "export TERM=dumb; export COLUMNS=120; export LINES=40; solana-keygen new --no-bip39-passphrase --outfile '$wslPath'$forceArg"
        $wslArgs = @()
        if (-not [string]::IsNullOrWhiteSpace($WslDistro)) {
            $wslArgs += @("-d", $WslDistro.Trim())
        }
        $wslArgs += @("-e", "bash", "-lc", $cmd)
        $oldEap = $ErrorActionPreference
        try {
            $ErrorActionPreference = "Continue"
            & wsl.exe @wslArgs 2>&1 | Where-Object { $_ -notmatch "screen size is bogus" } | Out-File -FilePath $tmp -Encoding utf8
        } finally {
            $ErrorActionPreference = $oldEap
        }
        if ($LASTEXITCODE -ne 0) {
            Fail ("WSL solana-keygen failed (exit=$LASTEXITCODE). Ensure solana-keygen works in WSL: wsl.exe -d '" + $WslDistro + "' -e bash -lc 'solana-keygen --version'")
        }
    }
} finally {
    if (Test-Path $tmp) { Remove-Item -Force $tmp -ErrorAction SilentlyContinue }
}

if (-not (Test-Path $outPath)) {
    Fail "Keypair file was not created: $outPath"
}

if (-not $useWsl) {
    $pubkey = (& solana-keygen pubkey $outPath) | Select-Object -First 1
} else {
    $wslPath = Convert-WindowsPathToWsl $outPath
    $cmd = "export TERM=dumb; export COLUMNS=120; export LINES=40; solana-keygen pubkey '$wslPath'"
    $wslArgs = @()
    if (-not [string]::IsNullOrWhiteSpace($WslDistro)) {
        $wslArgs += @("-d", $WslDistro.Trim())
    }
    $wslArgs += @("-e", "bash", "-lc", $cmd)
    $oldEap = $ErrorActionPreference
    try {
        $ErrorActionPreference = "Continue"
        $pubkey = (& wsl.exe @wslArgs 2>&1 | Where-Object { $_ -notmatch "screen size is bogus" }) | Select-Object -First 1
    } finally {
        $ErrorActionPreference = $oldEap
    }
}
if ([string]::IsNullOrWhiteSpace($pubkey)) {
    Fail "Could not read pubkey from: $outPath"
}

Info ("Wrote keypair: " + $outPath)
Info ("Pubkey: " + $pubkey.Trim())
Info "Next: fund this pubkey from Phantom, then set KEYPAIR_PATH/SOLANA_KEYPAIR_PATH to this file for --execute."

