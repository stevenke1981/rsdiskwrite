<#
.SYNOPSIS
Packages rsdiskwrite.exe into dist/: a zip, a bare exe and SHA256SUMS.txt.
.EXAMPLE
pwsh -File scripts/package.ps1 -Version 0.1.0
#>
[CmdletBinding()]
param(
    [string]$Version,
    [string]$Target = 'x86_64-pc-windows-msvc',
    [string]$BinaryPath
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot

if (-not $Version) {
    $cargoToml = Get-Content -LiteralPath (Join-Path $root 'Cargo.toml') -Raw
    $pkgSection = [regex]::Match($cargoToml, '(?ms)^\[package\]\s*(.*?)(?=^\[|\z)').Groups[1].Value
    $m = [regex]::Match($pkgSection, '(?m)^version\s*=\s*"([^"]+)"')
    if (-not $m.Success) { throw 'Could not read version from Cargo.toml' }
    $Version = $m.Groups[1].Value
}

if (-not $BinaryPath) {
    $targetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $root 'target' }
    $BinaryPath = Join-Path $targetDir 'release\rsdiskwrite.exe'
}
$BinaryPath = [IO.Path]::GetFullPath([IO.Path]::Combine((Get-Location).ProviderPath, $BinaryPath))
if (-not (Test-Path -LiteralPath $BinaryPath -PathType Leaf)) {
    throw "Binary not found: $BinaryPath (run cargo build --release --locked first)"
}

$name = "rsdiskwrite-$Version-$Target"
$dist = Join-Path $root 'dist'
$stage = Join-Path $dist $name
$zip = Join-Path $dist "$name.zip"
$exe = Join-Path $dist "$name.exe"
$sums = Join-Path $dist 'SHA256SUMS.txt'

if (Test-Path -LiteralPath $dist) { Remove-Item -LiteralPath $dist -Recurse -Force }
New-Item -ItemType Directory -Path $stage -Force | Out-Null

Copy-Item -LiteralPath $BinaryPath -Destination (Join-Path $stage 'rsdiskwrite.exe')
foreach ($f in @('README.md', 'LICENSE')) {
    $src = Join-Path $root $f
    if (-not (Test-Path -LiteralPath $src -PathType Leaf)) { throw "Required file missing: $f" }
    Copy-Item -LiteralPath $src -Destination $stage
}
$changelog = Join-Path $root 'CHANGELOG.md'
if (Test-Path -LiteralPath $changelog -PathType Leaf) { Copy-Item -LiteralPath $changelog -Destination $stage }
$docs = Join-Path $root 'docs'
if (Test-Path -LiteralPath $docs -PathType Container) {
    Copy-Item -LiteralPath $docs -Destination (Join-Path $stage 'docs') -Recurse
}

Add-Type -AssemblyName System.IO.Compression
# Entries are written by hand so names use '/' (ZipFile.CreateFromDirectory emits '\' on .NET Framework / Windows PowerShell 5.1)
$zipStream = [IO.File]::Open($zip, [IO.FileMode]::Create)
$archive = New-Object System.IO.Compression.ZipArchive -ArgumentList $zipStream, ([System.IO.Compression.ZipArchiveMode]::Create)
try {
    foreach ($item in (Get-ChildItem -LiteralPath $stage -Recurse -File)) {
        $rel = $item.FullName.Substring($stage.Length).TrimStart([char[]]@('\', '/')) -replace '\\', '/'
        $entry = $archive.CreateEntry("$name/$rel", [System.IO.Compression.CompressionLevel]::Optimal)
        $inStream = [IO.File]::OpenRead($item.FullName)
        try { $outStream = $entry.Open(); try { $inStream.CopyTo($outStream) } finally { $outStream.Dispose() } } finally { $inStream.Dispose() }
    }
} finally {
    $archive.Dispose()
    $zipStream.Dispose()
}
Copy-Item -LiteralPath $BinaryPath -Destination $exe
Remove-Item -LiteralPath $stage -Recurse -Force

$lines = foreach ($file in @((Get-Item -LiteralPath $zip), (Get-Item -LiteralPath $exe))) {
    $hash = (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
    "$hash  $($file.Name)"
}
$lf = [string][char]10
$utf8NoBom = New-Object System.Text.UTF8Encoding -ArgumentList $false
[IO.File]::WriteAllText($sums, (($lines -join $lf) + $lf), $utf8NoBom)

Write-Host "Packaged $name into $dist"
Get-ChildItem -LiteralPath $dist -File | ForEach-Object { '{0,12}  {1}' -f $_.Length, $_.Name }
Write-Host '--- SHA256SUMS.txt ---'
Get-Content -LiteralPath $sums
