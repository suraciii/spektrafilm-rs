param([string]$BuildManifest = $env:BUILD_MANIFEST)
$ErrorActionPreference = 'Stop'

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
Set-Location $repoRoot
. "$PSScriptRoot/package_evidence.ps1"

$arch = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString().ToLowerInvariant()
$distRoot = if ($env:SPEKTRAFILM_DIST_DIR) {
    $env:SPEKTRAFILM_DIST_DIR
} else {
    Join-Path $repoRoot "dist\spektrafilm-windows-$arch"
}
$archive = "$distRoot.zip"

if (Test-Path -LiteralPath $distRoot) {
    Remove-Item -LiteralPath $distRoot -Recurse -Force
}
if (Test-Path -LiteralPath $archive) {
    Remove-Item -LiteralPath $archive -Force
}
New-Item -ItemType Directory -Force -Path $distRoot | Out-Null

foreach ($binary in @('spektrafilm-gui.exe', 'spektrafilm.exe', 'spektrafilm-f64.exe', 'decode_raw_gui.exe')) {
    $source = Get-BinarySource $binary
    if (-not (Test-Path -LiteralPath $source)) {
        throw "Missing release binary: $source"
    }
    Copy-Item -LiteralPath $source -Destination (Join-Path $distRoot $binary)
}

Copy-Item -LiteralPath (Join-Path $repoRoot 'data') -Destination (Join-Path $distRoot 'data') -Recurse
@"
Spektrafilm Windows $arch

Run spektrafilm-gui.exe for the desktop application.
The CLI and the f64 exporter are in this directory, together with data\.
The package uses the default wgpu/WGSL backend.
"@ | Set-Content -LiteralPath (Join-Path $distRoot 'README.txt') -Encoding UTF8

& python scripts/build_evidence.py --manifest $BuildManifest --package-root $distRoot
if ($LASTEXITCODE -ne 0) { throw "Package provenance failed" }

Compress-Archive -Path (Join-Path $distRoot '*') -DestinationPath $archive -Force
Write-Host "Package: $distRoot"
Write-Host "Archive: $archive"
