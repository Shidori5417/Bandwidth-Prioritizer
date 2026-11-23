<#
.SYNOPSIS
Builds and packages Bandwidth-Prioritizer (bwp) for Windows release.

.DESCRIPTION
This script compiles the project in release mode and copies the necessary
binaries and configuration files into a zip archive for distribution.
#>

$ErrorActionPreference = "Stop"

$ProjectRoot = $PSScriptRoot
$ReleaseDir = Join-Path $ProjectRoot "target\bwp-release"

# Derive the version from the workspace Cargo.toml so the archive name never goes stale.
$VersionMatch = Select-String -Path (Join-Path $ProjectRoot "Cargo.toml") -Pattern '^\s*version\s*=\s*"([^"]+)"' | Select-Object -First 1
if (-not $VersionMatch) { throw "Could not read the version from Cargo.toml" }
$Version = $VersionMatch.Matches[0].Groups[1].Value
$ZipFile = Join-Path $ProjectRoot "target\bwp-windows-v$Version.zip"

Write-Host "[*] Building Bandwidth-Prioritizer in release mode..." -ForegroundColor Cyan
Set-Location $ProjectRoot
cargo build --release

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
    "LICENSE-MIT"
)

foreach ($File in $FilesToPackage) {
    $SourcePath = Join-Path $ProjectRoot $File
    if (Test-Path $SourcePath) {
        Copy-Item -Path $SourcePath -Destination $ReleaseDir -Force
        Write-Host "  v Copied $File" -ForegroundColor Green
    } else {
        Write-Host "  x Missing file: $File" -ForegroundColor Red
    }
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
