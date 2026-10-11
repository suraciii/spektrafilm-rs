# Shared admission of Cargo build evidence for PowerShell producers.
if (-not $BuildManifest) {
    $evidenceDir = Join-Path ([System.IO.Path]::GetTempPath()) ("spektrafilm-build-" + [guid]::NewGuid())
    & python scripts/build_evidence.py --out-dir $evidenceDir --profile release
    if ($LASTEXITCODE -ne 0) { throw "Attested build failed" }
    $BuildManifest = Join-Path $evidenceDir 'manifest.json'
}
& python scripts/build_evidence.py --manifest $BuildManifest
if ($LASTEXITCODE -ne 0) { throw "Build manifest verification failed" }
$build = Get-Content -LiteralPath $BuildManifest -Raw | ConvertFrom-Json
function Get-BinarySource([string]$Name) {
    $label = switch ($Name.Replace('.exe', '')) {
        'spektrafilm-gui' { 'gui' }
        'spektrafilm' { 'cli' }
        'spektrafilm-f64' { 'exporter' }
        'decode_raw_gui' { 'raw_helper' }
    }
    return $build.binaries.$label.path
}
