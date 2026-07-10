# Import the release code-signing certificate and generate the Tauri config used
# for inside-out Windows signing (main executable, NSIS uninstaller, installer).
#
# This script is intentionally release-only: missing credentials are fatal. Pull
# request workflows must never receive the PFX secret; they keep building unsigned
# installers and exercise the same NSIS template in optional verification mode.

[CmdletBinding()]
param(
    [string]$CertificateBase64 = $env:WINDOWS_CERTIFICATE,
    [string]$CertificatePassword = $env:WINDOWS_CERTIFICATE_PASSWORD,
    [string]$TimestampUrl = $(if ($env:WINDOWS_TIMESTAMP_URL) { $env:WINDOWS_TIMESTAMP_URL } else { "http://timestamp.digicert.com" }),
    [string]$ConfigPath = $(Join-Path ([System.IO.Path]::GetTempPath()) "tauri-authenticode.conf.json"),
    [string]$Stage = "sign-prepare"
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Fail-Stage([string]$Message) {
    Write-Host "::error::[$Stage] $Message"
    throw "[$Stage] $Message"
}

if ([string]::IsNullOrWhiteSpace($CertificateBase64)) {
    Fail-Stage "WINDOWS_CERTIFICATE is required for a release build"
}
if ([string]::IsNullOrWhiteSpace($TimestampUrl)) {
    Fail-Stage "WINDOWS_TIMESTAMP_URL must resolve to an RFC3161 timestamp service"
}

$timestamp = $null
try {
    $timestamp = [Uri]$TimestampUrl
}
catch {
    Fail-Stage "WINDOWS_TIMESTAMP_URL is not a valid absolute URL"
}
if (-not $timestamp.IsAbsoluteUri -or $timestamp.Scheme -notin @("http", "https")) {
    Fail-Stage "WINDOWS_TIMESTAMP_URL must use http or https"
}

$tempDir = Join-Path ([System.IO.Path]::GetTempPath()) ("cam-authenticode-" + [guid]::NewGuid().ToString("n"))
New-Item -ItemType Directory -Path $tempDir | Out-Null
$pfxPath = Join-Path $tempDir "codesign.pfx"

try {
    [IO.File]::WriteAllBytes($pfxPath, [Convert]::FromBase64String($CertificateBase64.Trim()))
    $securePass = if ([string]::IsNullOrEmpty($CertificatePassword)) {
        New-Object System.Security.SecureString
    }
    else {
        ConvertTo-SecureString -String $CertificatePassword -AsPlainText -Force
    }

    $imported = @(Import-PfxCertificate `
        -FilePath $pfxPath `
        -CertStoreLocation Cert:\CurrentUser\My `
        -Password $securePass)
    $certificate = $imported |
        Where-Object { $_.HasPrivateKey } |
        Select-Object -First 1
    if (-not $certificate) {
        Fail-Stage "the imported PFX contains no certificate with a private key"
    }

    $thumbprint = $certificate.Thumbprint.Replace(" ", "").ToUpperInvariant()
    $configDir = Split-Path -Parent $ConfigPath
    if ($configDir) {
        New-Item -ItemType Directory -Path $configDir -Force | Out-Null
    }
    @{
        bundle = @{
            windows = @{
                certificateThumbprint = $thumbprint
                digestAlgorithm = "sha256"
                timestampUrl = $TimestampUrl
                tsp = $true
            }
        }
    } | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $ConfigPath -Encoding utf8

    Write-Host "[$Stage] imported subject=$($certificate.Subject) thumbprint=$thumbprint"
    Write-Host "[$Stage] generated Tauri Authenticode config: $ConfigPath"

    if ($env:GITHUB_ENV) {
        "TAURI_AUTHENTICODE_CONFIG=$ConfigPath" | Out-File -FilePath $env:GITHUB_ENV -Append -Encoding utf8
        "WINDOWS_CERTIFICATE_THUMBPRINT=$thumbprint" | Out-File -FilePath $env:GITHUB_ENV -Append -Encoding utf8
        "WINDOWS_CERTIFICATE_TEMP_DIR=$tempDir" | Out-File -FilePath $env:GITHUB_ENV -Append -Encoding utf8
    }
    if ($env:GITHUB_OUTPUT) {
        "thumbprint=$thumbprint" | Out-File -FilePath $env:GITHUB_OUTPUT -Append -Encoding utf8
        "config=$ConfigPath" | Out-File -FilePath $env:GITHUB_OUTPUT -Append -Encoding utf8
    }
}
catch {
    if (Test-Path -LiteralPath $tempDir) {
        Remove-Item -LiteralPath $tempDir -Recurse -Force -ErrorAction SilentlyContinue
    }
    throw
}

return
