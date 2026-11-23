# Fetch the Everything command-line tool (es.exe) into the gitignored tools/es/ cache.
#
# The app downloads es.exe at runtime (exec::fetch_and_install) and the settings page can
# also install a manually downloaded file. This script is a developer convenience: it
# refreshes the ignored tools/es/ cache and prints the hashes needed to update ALLOWLIST
# in crates/core/src/exec.rs after a deliberate version bump.
#
# ASCII-ONLY: PowerShell 5.1 decodes .ps1 as ANSI; non-ASCII bytes can swallow a newline
# and silently null out later variables (see scripts/verify.ps1 header).
param(
  [string]$Url = 'https://www.voidtools.com/es.zip',
  [switch]$Force
)

$ErrorActionPreference = 'Stop'
# param() must be the first statement in the file, so the self-guard goes right after it.
$self = Get-Content -LiteralPath $PSCommandPath -Raw
if ($self -match '[^\x00-\x7F]') {
  Write-Host 'ERROR: scripts/fetch-es.ps1 must stay ASCII-only' -ForegroundColor Red
  exit 2
}

$root = Split-Path -Parent $PSScriptRoot
$dir = Join-Path $root 'tools\es'
New-Item -ItemType Directory -Force -Path $dir | Out-Null
$zipPath = Join-Path $dir 'es.zip'
$exePath = Join-Path $dir 'es.exe'

if ((Test-Path $exePath) -and -not $Force) {
  $cur = (Get-FileHash -Algorithm SHA256 -LiteralPath $exePath).Hash.ToLowerInvariant()
  Write-Host "already present: $exePath"
  Write-Host "  sha256 = $cur"
  Write-Host 'pass -Force to re-download'
  exit 0
}

Write-Host "downloading $Url"
$tmp = Join-Path $env:TEMP ('es-' + [guid]::NewGuid().ToString('N') + '.zip')
# TLS through .NET; Invoke-WebRequest on 5.1 is slow and re-encodes bodies
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
$wc = New-Object System.Net.WebClient
$wc.DownloadFile($Url, $tmp)
$zipHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $tmp).Hash.ToLowerInvariant()
Copy-Item -LiteralPath $tmp -Destination $zipPath -Force
# temp cleanup is best-effort: workspace file protection may block deletion
Remove-Item -LiteralPath $tmp -Force -ErrorAction SilentlyContinue

$stage = Join-Path $env:TEMP ('es-x-' + [guid]::NewGuid().ToString('N'))
Expand-Archive -LiteralPath $zipPath -DestinationPath $stage -Force
$found = Get-ChildItem -LiteralPath $stage -Recurse -Filter 'es.exe' | Select-Object -First 1
if (-not $found) { throw 'es.exe not found inside the archive' }
Copy-Item -LiteralPath $found.FullName -Destination $exePath -Force
Remove-Item -LiteralPath $stage -Recurse -Force -ErrorAction SilentlyContinue

$exeHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $exePath).Hash.ToLowerInvariant()
$version = (& $exePath -version) -join ''
$size = (Get-Item -LiteralPath $exePath).Length

Write-Host ''
Write-Host "es.exe  size=$size  version=$version"
Write-Host "  zip sha256 = $zipHash"
Write-Host "  exe sha256 = $exeHash"
Write-Host ''
Write-Host 'Now update ALLOWLIST.sha256 in crates/core/src/exec.rs to the exe hash above,'
Write-Host 'then run:  scripts/cargo.ps1 -Cmd "test -p efto-core --lib exec"'
