# Build + run the desktop app in debug, with the repo's example plugins registered.
# ASCII-only (PowerShell 5.1 reads .ps1 as ANSI).
param([string[]]$AppArgs = @())
. (Join-Path $PSScriptRoot 'vsenv.ps1')

$root = Split-Path -Parent $PSScriptRoot
# Point the launcher at the in-repo example plugins so `cargo run` is useful immediately.
if (-not $env:EFTO_PLUGIN_DIRS) {
  $env:EFTO_PLUGIN_DIRS = (Join-Path $root 'plugins')
}
if (-not $env:RUST_LOG) { $env:RUST_LOG = 'info' }

Write-Host "plugin dirs: $env:EFTO_PLUGIN_DIRS"
if ($AppArgs.Count -gt 0) {
  & cargo run -p efto-app -- $AppArgs
} else {
  & cargo run -p efto-app
}
exit $LASTEXITCODE
