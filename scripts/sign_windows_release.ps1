param(
    [Parameter(Mandatory = $true)]
    [string]$FilePath,
    [string]$ConfigPath = "signing/windows-signing.env"
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Read-KeyValueConfig {
    param([string]$Path)

    $map = @{}
    foreach ($line in Get-Content -LiteralPath $Path) {
        $trimmed = $line.Trim()
        if ([string]::IsNullOrWhiteSpace($trimmed)) { continue }
        if ($trimmed.StartsWith("#")) { continue }

        $idx = $trimmed.IndexOf("=")
        if ($idx -lt 1) { continue }

        $key = $trimmed.Substring(0, $idx).Trim()
        $value = $trimmed.Substring($idx + 1).Trim()

        if ($value.Length -ge 2) {
            if (($value.StartsWith('"') -and $value.EndsWith('"')) -or ($value.StartsWith("'") -and $value.EndsWith("'"))) {
                $value = $value.Substring(1, $value.Length - 2)
            }
        }

        if (-not [string]::IsNullOrWhiteSpace($key)) {
            $map[$key] = $value
        }
    }
    return $map
}

function Find-SignTool {
    if ($env:SIGNTOOL_PATH -and (Test-Path -LiteralPath $env:SIGNTOOL_PATH)) {
        return $env:SIGNTOOL_PATH
    }

    $fromPath = Get-Command signtool.exe -ErrorAction SilentlyContinue
    if ($fromPath) {
        return $fromPath.Source
    }

    $kitsRoot = Join-Path ${env:ProgramFiles(x86)} "Windows Kits\10\bin"
    if (Test-Path -LiteralPath $kitsRoot) {
        $candidate = Get-ChildItem -LiteralPath $kitsRoot -Recurse -Filter signtool.exe |
            Sort-Object FullName -Descending |
            Select-Object -First 1
        if ($candidate) {
            return $candidate.FullName
        }
    }

    throw "signtool.exe not found. Install Windows SDK Signing Tools or set SIGNTOOL_PATH."
}

if (-not (Test-Path -LiteralPath $FilePath)) {
    throw "File not found: $FilePath"
}

if (-not (Test-Path -LiteralPath $ConfigPath)) {
    throw "Signing config not found: $ConfigPath"
}

$config = Read-KeyValueConfig -Path $ConfigPath

$enabled = $config["HUMAI_SIGN_ENABLED"]
if ($enabled -and @("1", "true", "yes", "on") -notcontains $enabled.ToLowerInvariant()) {
    Write-Host "[sign] HUMAI_SIGN_ENABLED is not true; skipping signing."
    exit 0
}

$pfxPath = $config["HUMAI_SIGN_PFX_PATH"]
$pfxPassword = $config["HUMAI_SIGN_PFX_PASSWORD"]
$timestampUrl = $config["HUMAI_SIGN_TIMESTAMP_URL"]
if ([string]::IsNullOrWhiteSpace($timestampUrl)) {
    $timestampUrl = "http://timestamp.digicert.com"
}

if ([string]::IsNullOrWhiteSpace($pfxPath)) {
    throw "HUMAI_SIGN_PFX_PATH is required in $ConfigPath"
}
if ([string]::IsNullOrWhiteSpace($pfxPassword)) {
    throw "HUMAI_SIGN_PFX_PASSWORD is required in $ConfigPath"
}

if (-not (Test-Path -LiteralPath $pfxPath)) {
    throw "PFX file not found: $pfxPath"
}

$signtool = Find-SignTool
Write-Host "[sign] Using signtool: $signtool"
Write-Host "[sign] Signing file: $FilePath"

& $signtool sign /fd SHA256 /td SHA256 /tr $timestampUrl /f $pfxPath /p $pfxPassword /v $FilePath
if ($LASTEXITCODE -ne 0) {
    throw "signtool sign failed with exit code $LASTEXITCODE"
}

Write-Host "[sign] Verifying signature..."
& $signtool verify /pa /v $FilePath
if ($LASTEXITCODE -ne 0) {
    throw "signtool verify failed with exit code $LASTEXITCODE"
}

Write-Host "[sign] Signature OK."
