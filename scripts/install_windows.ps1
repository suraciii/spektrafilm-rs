param(
    [string]$InstallRoot = $env:SPEKTRAFILM_INSTALL_ROOT,
    [string]$BuildManifest = $env:BUILD_MANIFEST
)

$ErrorActionPreference = 'Stop'

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
Set-Location $repoRoot
. "$PSScriptRoot/package_evidence.ps1"

if ([string]::IsNullOrWhiteSpace($InstallRoot)) {
    if ([string]::IsNullOrWhiteSpace($env:LOCALAPPDATA)) {
        throw 'LOCALAPPDATA is not set; pass -InstallRoot or set SPEKTRAFILM_INSTALL_ROOT.'
    }
    $InstallRoot = Join-Path $env:LOCALAPPDATA 'Spektrafilm'
}

New-Item -ItemType Directory -Force -Path $InstallRoot | Out-Null

foreach ($binary in @('spektrafilm-gui.exe', 'spektrafilm.exe', 'spektrafilm-f64.exe', 'decode_raw_gui.exe')) {
    $source = Get-BinarySource $binary
    if (-not (Test-Path -LiteralPath $source)) {
        throw "Missing release binary: $source"
    }
    Copy-Item -LiteralPath $source -Destination (Join-Path $InstallRoot $binary) -Force
}

$dataRoot = Join-Path $InstallRoot 'data'
if (Test-Path -LiteralPath $dataRoot) {
    Remove-Item -LiteralPath $dataRoot -Recurse -Force
}
Copy-Item -LiteralPath (Join-Path $repoRoot 'data') -Destination $dataRoot -Recurse
& python scripts/build_evidence.py --manifest $BuildManifest --package-root $InstallRoot
if ($LASTEXITCODE -ne 0) { throw "Installation provenance failed" }

Write-Host "Installed Spektrafilm to: $InstallRoot"
Write-Host "Launch: $InstallRoot\spektrafilm-gui.exe"
Write-Host 'The install directory is intentionally not added to PATH; add it manually if CLI commands should be global.'
