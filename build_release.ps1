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
$ZipFile = Join-Path $ProjectRoot "target\bwp-windows-v1.0.0.zip"

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
    "target\release\WinDivert.dll",
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
