$ErrorActionPreference = 'Stop'

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
Set-Location $repoRoot

$arch = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString().ToLowerInvariant()
$distRoot = if ($env:SPEKTRAFILM_DIST_DIR) {
    $env:SPEKTRAFILM_DIST_DIR
} else {
    Join-Path $repoRoot "dist\spektrafilm-windows-$arch"
}
$archive = "$distRoot.zip"

& (Join-Path $PSScriptRoot 'package_windows_app.ps1') -BuildTargetDir 'target' -DistDir $distRoot
if (-not (Test-Path -LiteralPath $archive)) { throw "Native package archive missing: $archive" }
Write-Host "Package: $distRoot"
Write-Host "Archive: $archive"
