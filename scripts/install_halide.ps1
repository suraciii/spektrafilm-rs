param([Parameter(Mandatory=$true)][string]$Prefix)
$ErrorActionPreference = 'Stop'
$revision = 'b629c80de18f1534ec71fddd8b567aa7027a0876'
$archive = "Halide-21.0.0-x86-64-windows-$revision.zip"
$url = "https://github.com/halide/Halide/releases/download/v21.0.0/$archive"
$digest = '4efec94b7c8958b1ae0125a73245a148e4c98dbb54f2678bc34c6abe36ee899a'
$work = Join-Path ([IO.Path]::GetTempPath()) ([guid]::NewGuid().ToString())
New-Item -ItemType Directory -Path $work | Out-Null
try {
    $download = Join-Path $work $archive
    Invoke-WebRequest -Uri $url -OutFile $download
    if ((Get-FileHash -Algorithm SHA256 -LiteralPath $download).Hash.ToLowerInvariant() -ne $digest) {
        throw 'Official Halide archive SHA256 mismatch'
    }
    Expand-Archive -LiteralPath $download -DestinationPath (Join-Path $work 'extracted')
    $roots = @(Get-ChildItem -LiteralPath (Join-Path $work 'extracted') -Directory)
    if ($roots.Count -ne 1) { throw 'Unexpected official Halide archive layout' }
    New-Item -ItemType Directory -Force -Path $Prefix | Out-Null
    Get-ChildItem -LiteralPath $roots[0].FullName | Copy-Item -Destination $Prefix -Recurse -Force
    if (-not (Test-Path (Join-Path $Prefix 'include\Halide.h'))) { throw 'Official Halide header missing' }
    if (-not (Test-Path (Join-Path $Prefix 'share\doc\Halide\LICENSE.txt'))) { throw 'Official Halide notices missing' }
    "version=21.0.0`nrevision=$revision`nurl=$url`nsha256=$digest" | Set-Content -LiteralPath (Join-Path $Prefix 'Halide-provenance.txt')
    Write-Host "Installed Halide 21.0.0 in $Prefix"
} finally { Remove-Item -LiteralPath $work -Recurse -Force }
