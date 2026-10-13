param(
    [string]$InstallRoot = $env:SPEKTRAFILM_INSTALL_ROOT
)

$ErrorActionPreference = 'Stop'

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
Set-Location $repoRoot

if ([string]::IsNullOrWhiteSpace($InstallRoot)) {
    if ([string]::IsNullOrWhiteSpace($env:LOCALAPPDATA)) {
        throw 'LOCALAPPDATA is not set; pass -InstallRoot or set SPEKTRAFILM_INSTALL_ROOT.'
    }
    $InstallRoot = Join-Path $env:LOCALAPPDATA 'Spektrafilm'
}

New-Item -ItemType Directory -Force -Path $InstallRoot | Out-Null

$arch = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString().ToLowerInvariant()
$packageRoot = if ($env:SPEKTRAFILM_DIST_DIR) { $env:SPEKTRAFILM_DIST_DIR } else { Join-Path $repoRoot "dist\spektrafilm-windows-$arch" }
& (Join-Path $PSScriptRoot 'package_windows.ps1')
Get-ChildItem -LiteralPath $packageRoot | ForEach-Object {
    Copy-Item -LiteralPath $_.FullName -Destination $InstallRoot -Recurse -Force
}

Write-Host "Installed Spektrafilm to: $InstallRoot"
Write-Host "Launch: $InstallRoot\spektrafilm-gui.exe"
Write-Host "Benchmark: $InstallRoot\spektrafilm-bench.exe and $InstallRoot\spektrafilm-bench-fixtures.exe"
Write-Host 'The install directory is intentionally not added to PATH; add it manually if CLI commands should be global.'
