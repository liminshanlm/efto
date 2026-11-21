# One-time toolchain bootstrap for efto on Windows.
# ASCII-only: Windows PowerShell 5.1 reads .ps1 as ANSI, UTF-8 comments break parsing.
# Requires: an interactive desktop (the VS installer triggers UAC).
$ErrorActionPreference = 'Stop'

function Step($msg) { Write-Host "==> $msg" -ForegroundColor Cyan }
function Have($cmd) { return [bool](Get-Command $cmd -ErrorAction SilentlyContinue) }

# --- 1. Rust via rustup, using the rsproxy mirror (static.rust-lang.org may be blocked) ---
$cargoBin = Join-Path $env:USERPROFILE '.cargo\bin'
if (-not (Test-Path (Join-Path $cargoBin 'cargo.exe'))) {
  Step 'Installing rustup (mirror: rsproxy.cn)'
  $tmp = Join-Path $env:TEMP 'rustup-init-efto.exe'
  curl.exe -sSL -o $tmp 'https://rsproxy.cn/rustup/dist/x86_64-pc-windows-msvc/rustup-init.exe'
  if ($LASTEXITCODE -ne 0 -or -not (Test-Path $tmp)) { throw 'rustup-init download failed' }
  $env:RUSTUP_DIST_SERVER = 'https://rsproxy.cn'
  $env:RUSTUP_UPDATE_ROOT = 'https://rsproxy.cn/rustup'
  & $tmp -y --no-modify-path --profile minimal --default-toolchain stable-x86_64-pc-windows-msvc
  if ($LASTEXITCODE -ne 0) { throw "rustup-init exited $LASTEXITCODE" }
  Remove-Item $tmp -ErrorAction SilentlyContinue
} else {
  Step 'rustup already present'
}

# --- 2. MSVC toolset + Windows SDK ---
$VsDev = 'C:\BuildTools-MSVC\Common7\Tools\VsDevCmd.bat'
$altDev = 'C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\Common7\Tools\VsDevCmd.bat'
if (-not (Test-Path $VsDev) -and (Test-Path $altDev)) { $VsDev = $altDev }
if (-not (Test-Path $VsDev)) {
  Step 'Installing VS Build Tools C++ workload (a UAC prompt will appear - click Yes)'
  $boot = Join-Path $env:TEMP 'vs_buildtools-efto.exe'
  curl.exe -sSL -o $boot 'https://aka.ms/vs/17/release/vs_buildtools.exe'
  if (-not (Test-Path $boot)) { throw 'vs_buildtools.exe download failed' }
  $a = @(
    '--quiet', '--wait', '--norestart', '--nocache',
    '--installPath', 'C:\BuildTools-MSVC',
    '--add', 'Microsoft.VisualStudio.Workload.VCTools',
    '--includeRecommended'
  )
  $p = Start-Process -FilePath $boot -ArgumentList $a -Verb RunAs -PassThru -Wait
  Write-Host "installer exit: $($p.ExitCode)"
  if (-not (Test-Path $VsDev)) {
    throw "VsDevCmd.bat still missing after installer run=$($p.ExitCode). Open 'Visual Studio Installer' -> Modify -> check 'Desktop development with C++'."
  }
} else {
  Step 'MSVC toolset already present'
}

# --- 3. V8 prebuilt artifact (GitHub direct downloads are often blocked here) ---
$root = Split-Path -Parent $PSScriptRoot
$lib = Join-Path $root 'vendor\rusty_v8\rusty_v8_release_x86_64-pc-windows-msvc.lib.gz'
if (-not (Test-Path $lib)) {
  Step 'Fetching rusty_v8 prebuilt static lib (via mirror)'
  New-Item -ItemType Directory -Force -Path (Split-Path $lib) | Out-Null
  $url = 'https://gh-proxy.com/https://github.com/denoland/rusty_v8/releases/download/v152.2.0/rusty_v8_release_x86_64-pc-windows-msvc.lib.gz'
  curl.exe -sSL -o $lib $url
  if ($LASTEXITCODE -ne 0 -or (Get-Item $lib).Length -lt 1000000) {
    throw "V8 artifact download failed. Set RUSTY_V8_MIRROR in .cargo/config.toml to a reachable mirror."
  }
} else {
  Step 'V8 artifact already vendored'
}

# --- 4. Icons (checked in, but regenerate if missing) ---
$ico = Join-Path $root 'crates\app\icons\icon.ico'
if (-not (Test-Path $ico) -and (Have node)) {
  Step 'Generating icons'
  node (Join-Path $root 'scripts\gen-icons.js')
}

Step 'Done. Next:'
Write-Host '  powershell -ExecutionPolicy Bypass -File scripts/cargo.ps1 -Cmd "test --workspace"'
Write-Host '  powershell -ExecutionPolicy Bypass -File scripts/dev-run.ps1'
