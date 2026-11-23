# Package plugins as standalone zip files for distribution / manual install.
#
# ASCII-ONLY by convention: Windows PowerShell 5.1 decodes .ps1 as ANSI, so
# non-ASCII literals can corrupt statements. Keep this file ASCII.
#
# The archive root is the plugin directory itself (efto-plugin.json at top),
# which is what the settings-page import and a manual unzip into
# ~/.efto/plugins both expect.
#
#   powershell -ExecutionPolicy Bypass -File scripts/package-plugins.ps1
#   powershell -ExecutionPolicy Bypass -File scripts/package-plugins.ps1 -Only markdown,timestamp-ts
param(
    [string]$OutDir = '',
    [string[]]$Only = @()
)
$ErrorActionPreference = 'Stop'

$repo = Split-Path -Parent $PSScriptRoot
$pluginsRoot = Join-Path $repo 'plugins'
if (-not (Test-Path -LiteralPath $pluginsRoot)) { throw "plugins directory not found: $pluginsRoot" }
if (-not $OutDir) { $OutDir = Join-Path $repo 'dist\plugins' }

New-Item -ItemType Directory -Force -Path $OutDir | Out-Null

# `powershell -File` passes "-Only a,b" as one string, so split on commas too.
$wanted = @()
foreach ($entry in $Only) { $wanted += ($entry -split ',' | ForEach-Object { $_.Trim() } | Where-Object { $_ }) }

$count = 0
Get-ChildItem -LiteralPath $pluginsRoot -Directory | ForEach-Object {
    $dir = $_
    $manifestPath = Join-Path $dir.FullName 'efto-plugin.json'
    if (-not (Test-Path -LiteralPath $manifestPath)) { return }
    # Manifests are UTF-8 without BOM; PS 5.1 defaults to ANSI and would corrupt CJK.
    $manifest = Get-Content -LiteralPath $manifestPath -Raw -Encoding UTF8 | ConvertFrom-Json
    if (-not $manifest.name) { throw "missing name field: $manifestPath" }
    if ($wanted.Count -gt 0 -and $wanted -notcontains $manifest.name) { return }

    $version = if ($manifest.version) { $manifest.version } else { '0.0.0' }
    $zipPath = Join-Path $OutDir ("{0}-{1}.zip" -f $manifest.name, $version)
    if (Test-Path -LiteralPath $zipPath) { Remove-Item -LiteralPath $zipPath -Force }

    # Stage first so the archive root is the plugin itself, not plugins\<name>\.
    $stage = Join-Path ([System.IO.Path]::GetTempPath()) ('efto-pack-' + [guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Force -Path $stage | Out-Null
    try {
        Copy-Item -Path (Join-Path $dir.FullName '*') -Destination $stage -Recurse -Force
        Compress-Archive -Path (Join-Path $stage '*') -DestinationPath $zipPath -CompressionLevel Optimal
    } finally {
        Remove-Item -LiteralPath $stage -Recurse -Force -ErrorAction SilentlyContinue
    }

    $size = [math]::Round((Get-Item -LiteralPath $zipPath).Length / 1KB, 1)
    Write-Host ("packed {0} {1} -> {2} ({3} KB)" -f $manifest.name, $version, $zipPath, $size)
    $count++
}

if ($count -eq 0) { throw "no plugin packaged (check -Only and plugins/*/efto-plugin.json)" }
Write-Host ("done: {0} plugin(s) -> {1}" -f $count, $OutDir)
