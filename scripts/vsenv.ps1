# efto dev environment bootstrap.
# Imports the MSVC / Windows SDK environment, then removes Git Bash paths that
# shadow the real linker (Git ships a coreutils link.exe in /usr/bin).
# Keep this file ASCII-only: Windows PowerShell 5.1 reads .ps1 as ANSI, which
# mangles UTF-8 comments and breaks string terminators.
$ErrorActionPreference = 'Stop'

$VsDevCmd = 'C:\BuildTools-MSVC\Common7\Tools\VsDevCmd.bat'
if (-not (Test-Path $VsDevCmd)) {
  throw "VsDevCmd.bat not found at $VsDevCmd - install the VS Build Tools C++ workload first"
}

# Run VsDevCmd in cmd and capture the resulting environment block.
$dump = & cmd.exe /c "`"$VsDevCmd`" -arch=amd64 -host_arch=amd64 -no_logo && set"
foreach ($line in $dump) {
  if ($line -match '^(.*?)=(.*)$') {
    Set-Item -Path ('Env:' + $matches[1]) -Value $matches[2]
  }
}

# Drop directories whose link.exe is not the MSVC linker.
$bad = @('\usr\bin', '\Git\usr\bin', '\mingw64\bin', '\usr\local\bin')
$keep = @()
foreach ($p in ($env:PATH -split ';')) {
  $drop = $false
  foreach ($b in $bad) {
    if ($p -like ('*' + $b + '*')) { $drop = $true }
  }
  if (-not $drop) { $keep += $p }
}
$env:PATH = ($keep -join ';')

# rustup was installed with --no-modify-path, so add it explicitly.
$cargoBin = Join-Path $env:USERPROFILE '.cargo\bin'
if ((Test-Path $cargoBin) -and ($env:PATH -notlike ('*' + $cargoBin + '*'))) {
  $env:PATH = $cargoBin + ';' + $env:PATH
}

# Keep build artifacts inside the project (this project lives off the system drive).
# Derived from $PSScriptRoot instead of a hardcoded absolute path: a hardcoded path
# would silently stop matching if the folder moves -- Test-Path false means
# CARGO_TARGET_DIR never gets set and artifacts land elsewhere.
if (-not $env:CARGO_TARGET_DIR) {
  $projRoot = Split-Path -Parent $PSScriptRoot
  $env:CARGO_TARGET_DIR = (Join-Path $projRoot 'target')
}

if ($env:EFTO_VERBOSE) {
  foreach ($tool in @('link.exe', 'cl.exe', 'rc.exe')) {
    $src = (Get-Command $tool -ErrorAction SilentlyContinue).Source
    Write-Host ('{0} -> {1}' -f $tool, $src)
  }
  Write-Host ('INCLUDE set: ' + [bool]$env:INCLUDE)
}
