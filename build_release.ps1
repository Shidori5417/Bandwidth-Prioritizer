<#
.SYNOPSIS
Builds and packages Bandwidth-Prioritizer (bwp) for Windows release.

.DESCRIPTION
This script compiles the project in release mode and copies the necessary
binaries and configuration files into a zip archive for distribution.

.PARAMETER WithWinDivert
Builds the EXPERIMENTAL packet shaper (Cargo feature `experimental-windivert`). Downloads
the official WinDivert 2.2.2 release (checksum verified), links against it and ships
WinDivert.dll, WinDivert64.sys and the WinDivert license in the zip (name suffix
"-windivert"). Without this switch no WinDivert file is built or packaged.
#>
param(
    [switch]$WithWinDivert
)

$ErrorActionPreference = "Stop"

$ProjectRoot = $PSScriptRoot
$ReleaseDir = Join-Path $ProjectRoot "target\bwp-release"

# Derive the version from the workspace Cargo.toml so the archive name never goes stale.
$VersionMatch = Select-String -Path (Join-Path $ProjectRoot "Cargo.toml") -Pattern '^\s*version\s*=\s*"([^"]+)"' | Select-Object -First 1
if (-not $VersionMatch) { throw "Could not read the version from Cargo.toml" }
$Version = $VersionMatch.Matches[0].Groups[1].Value
$Suffix = if ($WithWinDivert) { "-windivert" } else { "" }
$ZipFile = Join-Path $ProjectRoot "target\bwp-windows-v$Version$Suffix.zip"

Set-Location $ProjectRoot

$WinDivertDir = $null
if ($WithWinDivert) {
    $WdVersion = "2.2.2"
    $WdSha256 = "63cb41763bb4b20f600b6de04e991a9c2be73279e317d4d82f237b150c5f3f15"
    $WdRoot = Join-Path $ProjectRoot "target\windivert"
    $WdZip = Join-Path $WdRoot "WinDivert-$WdVersion-A.zip"
    New-Item -ItemType Directory -Force -Path $WdRoot | Out-Null
    if (-not (Test-Path $WdZip)) {
        Write-Host "[*] Downloading WinDivert $WdVersion..." -ForegroundColor Cyan
        Invoke-WebRequest -Uri "https://github.com/basil00/WinDivert/releases/download/v$WdVersion/WinDivert-$WdVersion-A.zip" -OutFile $WdZip
    }
    $Hash = (Get-FileHash $WdZip -Algorithm SHA256).Hash.ToLower()
    if ($Hash -ne $WdSha256) { throw "WinDivert checksum mismatch: $Hash" }
    Expand-Archive -Path $WdZip -DestinationPath $WdRoot -Force
    $WinDivertSdk = Join-Path $WdRoot "WinDivert-$WdVersion-A"
    $WinDivertDir = Join-Path $WinDivertSdk "x64"
    $env:WINDIVERT_PATH = $WinDivertDir
}

Write-Host "[*] Building Bandwidth-Prioritizer in release mode..." -ForegroundColor Cyan
if ($WithWinDivert) {
    cargo build --release --features bwp-engine/experimental-windivert
} else {
    cargo build --release
}
if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }

Write-Host "[*] Preparing release folder at $ReleaseDir..." -ForegroundColor Cyan
if (Test-Path $ReleaseDir) {
    Remove-Item -Recurse -Force $ReleaseDir
}
New-Item -ItemType Directory -Force -Path $ReleaseDir | Out-Null

# Files to copy
$FilesToPackage = @(
    "target\release\bwp.exe",
    "target\release\bwp-engine.exe",
    "config.toml",
    "README.md",
    "LICENSE-MIT",
    "LICENSE-APACHE"
)

foreach ($File in $FilesToPackage) {
    $SourcePath = Join-Path $ProjectRoot $File
    if (Test-Path $SourcePath) {
        Copy-Item -Path $SourcePath -Destination $ReleaseDir -Force
        Write-Host "  v Copied $File" -ForegroundColor Green
    } else {
        throw "Missing file: $File"
    }
}

if ($WithWinDivert) {
    foreach ($File in @("WinDivert.dll", "WinDivert64.sys")) {
        Copy-Item -Path (Join-Path $WinDivertDir $File) -Destination $ReleaseDir -Force
        Write-Host "  v Copied $File (WinDivert $WdVersion)" -ForegroundColor Green
    }
    Copy-Item -Path (Join-Path $WinDivertSdk "LICENSE") -Destination (Join-Path $ReleaseDir "WinDivert-LICENSE.txt") -Force
}

Write-Host "[*] Creating zip archive..." -ForegroundColor Cyan
if (Test-Path $ZipFile) {
    Remove-Item -Force $ZipFile
}
Compress-Archive -Path "$ReleaseDir\*" -DestinationPath $ZipFile -Force

Write-Host "[*] Release packaged successfully: $ZipFile" -ForegroundColor Green

# Expose the archive path to GitHub Actions, if running there.
if ($env:GITHUB_OUTPUT) {
    "zip_path=$ZipFile" | Out-File -FilePath $env:GITHUB_OUTPUT -Append -Encoding utf8
    "version=$Version" | Out-File -FilePath $env:GITHUB_OUTPUT -Append -Encoding utf8
}
