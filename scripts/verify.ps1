# End-to-end verification without a GUI, driving `efto cli`.
#
# ASCII-ONLY by convention: Windows PowerShell 5.1 decodes .ps1 as ANSI (GBK on
# zh-CN), so non-ASCII literals silently corrupt string terminators and later
# statements break in confusing ways. Keep this file ASCII; the CJK test input is
# built from code points below.
$ErrorActionPreference = 'Stop'
# Self-guard: PowerShell 5.1 decodes .ps1 as ANSI, so a non-ASCII byte in this
# file can swallow a newline and null out a later variable -- which previously
# produced very confusing failures here. Fail loudly instead.
$self = Get-Content -LiteralPath $PSCommandPath -Raw
if ($self -match '[^\x00-\x7F]') {
  Write-Host 'ERROR: scripts/verify.ps1 must stay ASCII-only (see header comment)' -ForegroundColor Red
  exit 2
}

# Byte-level audit of every sibling .ps1, not just this one. Checking only ourselves
# left a hole: a Chinese comment added to vsenv.ps1 (dot-sourced by everything here)
# would mangle that file silently while this script still passed. Bytes, not decoded
# text, so the audit does not depend on how PowerShell chose to read them.
foreach ($ps in Get-ChildItem -LiteralPath $PSScriptRoot -Filter '*.ps1' -File) {
  $bytes = [System.IO.File]::ReadAllBytes($ps.FullName)
  $bad = 0
  for ($i = 0; $i -lt $bytes.Length; $i++) { if ($bytes[$i] -gt 0x7F) { $bad++ } }
  if ($bad -gt 0) {
    Write-Host ('ERROR: {0} has {1} non-ASCII byte(s); keep every .ps1 ASCII-only' -f $ps.Name, $bad) -ForegroundColor Red
    exit 2
  }
}
. (Join-Path $PSScriptRoot 'vsenv.ps1')

$root = Split-Path -Parent $PSScriptRoot
if (-not $env:EFTO_PLUGIN_DIRS) { $env:EFTO_PLUGIN_DIRS = (Join-Path $root 'plugins') }
$exe = Join-Path $env:CARGO_TARGET_DIR 'debug\efto.exe'
$work = Join-Path $root 'target\verify-tmp'
New-Item -ItemType Directory -Force -Path $work | Out-Null
$fail = 0

# "Notepad" in Chinese (U+8BB0 U+4E8B U+672C) from code points;
# "jsb" is its pinyin-initial query and is pure ASCII.
$notepad = [string]::Join('', @([char]0x8BB0, [char]0x4E8B, [char]0x672C))
$jsb = 'jsb'

# Run the CLI capturing stdout+stderr into a file and return the exit code.
# Going through a file (not a PowerShell pipe) avoids two 5.1 traps: stderr
# becoming an ErrorRecord, and a launched GUI child inheriting our console
# handle -- which makes the parent block until that child exits.
function Run([string[]]$cliArgs, [string]$outFile) {
  $line = '"' + $exe + '" ' + ($cliArgs -join ' ') + ' > "' + $outFile + '" 2>&1'
  cmd /c $line | Out-Null
  return $LASTEXITCODE
}

function ReadOut([string]$outFile) {
  if (Test-Path -LiteralPath $outFile) { return [string](Get-Content -LiteralPath $outFile -Raw) }
  return ''
}

function Check($name, [string[]]$cliArgs, [bool]$expectFail = $false) {
  Write-Host "--- $name" -ForegroundColor Cyan
  $f = Join-Path $work 'out.txt'
  $code = Run $cliArgs $f
  $text = ReadOut $f
  if ($text.Trim()) { Write-Host ($text.Trim()) }
  if ($expectFail) {
    if ($code -eq 0) { $script:fail++; Write-Host '  FAIL: expected a non-zero exit code' -ForegroundColor Red }
    else { Write-Host "  OK (exit $code, as expected)" -ForegroundColor Green }
  } elseif ($code -ne 0) {
    $script:fail++; Write-Host "  FAIL (exit $code)" -ForegroundColor Red
  } else {
    Write-Host '  OK' -ForegroundColor Green
  }
  return $text
}

Write-Host '==> build (debug)'
& cargo build -q -p efto-app
if ($LASTEXITCODE -ne 0) { throw 'build failed' }

# Frontend is embedded into the binary at compile time, and its CSS/JS interaction bugs
# (e.g. [hidden] overridden by display:flex swallowing all row clicks) cannot be
# auto-verified through the RayLink session -- so at least guard them statically.
Write-Host '==> frontend guard (node scripts/check-frontend.js)'
node (Join-Path $root 'scripts\check-frontend.js')
if ($LASTEXITCODE -ne 0) { $fail++; Write-Host '  FAIL: frontend guard reported a problem' -ForegroundColor Red }
else { Write-Host '  OK' -ForegroundColor Green }

# i18n: both dictionaries must agree on keys/placeholders, and every key used by
# pages or scripts must exist. Missing keys silently fall back to Chinese at
# runtime, so a static check is the only way to catch drift before release.
Write-Host '==> i18n guard (node scripts/check-i18n.js)'
node (Join-Path $root 'scripts\check-i18n.js')
if ($LASTEXITCODE -ne 0) { $fail++; Write-Host '  FAIL: i18n guard reported a problem' -ForegroundColor Red }
else { Write-Host '  OK' -ForegroundColor Green }

# Time -> timestamp conversion is calendar/timezone math. Unit tests alone would be
# Rust grading Rust, so this probe recomputes every expected epoch with Node's Date
# (the OS timezone database) and diffs it against what the binary actually returns.
Write-Host '==> time stamp cross-check (node scripts/check-time-probe.js)'
node (Join-Path $root 'scripts\check-time-probe.js')
if ($LASTEXITCODE -ne 0) { $fail++; Write-Host '  FAIL: time probe reported a problem' -ForegroundColor Red }
else { Write-Host '  OK' -ForegroundColor Green }

Check 'index built' @('cli', 'index')
$pluginsOut = Check 'plugins discovered' @('cli', 'plugins')
if ($pluginsOut -notmatch 'translate') {
  $fail++
  Write-Host '  FAIL: translate plugin not discovered' -ForegroundColor Red
}

Write-Host '--- search by pinyin initials (jsb) should find Notepad'
$f = Join-Path $work 'search-pinyin.txt'
$code = Run @('cli', 'search', $jsb, '--json') $f
$text = ReadOut $f
if ($code -eq 0 -and $text -match 'otepad') {
  Write-Host '  OK' -ForegroundColor Green
} else {
  $fail++
  Write-Host "  FAIL (exit $code, matched=$($text -match 'otepad'))" -ForegroundColor Red
}

Check 'search by Chinese app name' @('cli', 'search', ('"' + $notepad + '"'))

# Built-in calculator: expressions must produce a result row, and ordinary
# app names must NOT be hijacked by it.
foreach ($case in @(
    @{ q = '6*7'; expect = '42'; hijack = $false },
    @{ q = 'sqrt(16)+2^3'; expect = '12'; hijack = $false },
    @{ q = '1,000+1'; expect = '1,001'; hijack = $false },
    @{ q = '-2^2'; expect = '-4'; hijack = $false },
    @{ q = 'chrome'; expect = ''; hijack = $true },
    @{ q = '1700000000'; expect = ''; hijack = $true }
)) {
  $f = Join-Path $work 'calc.txt'
  $code = Run @('cli', 'search', ('"' + $case.q + '"')) $f
  $text = ReadOut $f
  Write-Host "--- query ""$($case.q)"""
  if ($case.hijack) {
    # These must fall through to app/no-result; a calc row here is a hijack
    if ($text -match 'calc') {
      $fail++; Write-Host "  FAIL: query was hijacked by the calculator" -ForegroundColor Red
    } else {
      Write-Host '  OK (not hijacked)' -ForegroundColor Green
    }
  } elseif ($code -eq 0 -and $text -match [regex]::Escape($case.expect)) {
    Write-Host "  OK -> $($case.expect)" -ForegroundColor Green
  } else {
    $fail++
    Write-Host "  FAIL: expected $($case.expect), exit $code" -ForegroundColor Red
    Write-Host (([string]$text).Trim())
  }
}

Check 'plugin-run: plain JS plugin' @('cli', 'plugin-run', 'hello-js', 'greet', 'hello-efto')

# timestamp-ts is a bidirectional converter now. A literal that carries its own offset
# must produce a fixed epoch pair (timezone-independent), and feeding that epoch back
# must land on the same calendar day again.
Write-Host '--- plugin-run: timestamp-ts time -> stamp' -ForegroundColor Cyan
$f = Join-Path $work 'ts-forward.txt'
$code = Run @('cli', 'plugin-run', 'timestamp-ts', 'ts', '2026-11-17T14:30:00+08:00') $f
$text = ReadOut $f
if ($code -eq 0 -and $text -match '"1794897000"' -and $text -match '"1794897000000"') {
  Write-Host '  OK (SWC strip + nested require + sec/ms rows)' -ForegroundColor Green
} else {
  $fail++
  Write-Host '  FAIL: expected the epoch pair of 2026-11-17T14:30:00+08:00' -ForegroundColor Red
  Write-Host (([string]$text).Trim())
}

Write-Host '--- plugin-run: timestamp-ts stamp -> time' -ForegroundColor Cyan
$f = Join-Path $work 'ts-reverse.txt'
$code = Run @('cli', 'plugin-run', 'timestamp-ts', 'ts', '1794897000') $f
$text = ReadOut $f
if ($code -eq 0 -and $text -match '2026-11-1[67]' -and $text -match 'ISO 8601') {
  Write-Host '  OK (local time + ISO rows)' -ForegroundColor Green
} else {
  $fail++
  Write-Host '  FAIL: expected a 2026-11-1x local time and an ISO 8601 row' -ForegroundColor Red
  Write-Host (([string]$text).Trim())
}
Check 'plugin-run: hosts plugin reads native capability' @('cli', 'plugin-run', 'hosts', 'hosts', 'hosts')
# translate: empty payload must give an inline hint (proves plugin loads + efto.output
# works) without touching the network -- real translation needs a model in settings.
Write-Host '--- plugin-run: translate with empty payload' -ForegroundColor Cyan
$f = Join-Path $work 'translate-empty.txt'
$code = Run @('cli', 'plugin-run', 'translate', 'fy') $f
$text = ReadOut $f
if ($code -eq 0 -and $text -match 'hello world') {
  Write-Host '  OK (inline hint via efto.output)' -ForegroundColor Green
} else {
  $fail++
  Write-Host '  FAIL: expected the inline "input text" hint' -ForegroundColor Red
  Write-Host (([string]$text).Trim())
}
Check 'plugin-run: unknown plugin must fail loudly' @('cli', 'plugin-run', 'nope', 'x', 'y') $true

# everything: pure JS logic (CSV parsing, noise filter, es arg building) runs
# against a captured real es sample, and checks every capability name the plugin calls
# actually exists in the prelude / UI SDK -- a typo'd capability fails silently at runtime.
# Everything search is a BUILT-IN inline source (like calc/clock): file rows are merged
# below app results in the main dropdown, with no magic prefix. This depends on Everything
# being installed and running, which the suite does not control -- no rows at all means
# SKIP (with the reason), never a pass.
Write-Host '==> everything inline source (real es.exe + Everything index)'
$f = Join-Path $work 'es.txt'
$code = Run @('cli', 'search', '"ext:exe"', '--kind', 'file') $f
$text = ReadOut $f
if ($text -match '\(everything\)') {
  if ($text -match 'RECYCLE') {
    $fail++; Write-Host '  FAIL: recycle-bin noise leaked into inline rows' -ForegroundColor Red
  } else {
    Write-Host '  OK (file rows merged below app results, no noise)' -ForegroundColor Green
  }
} elseif ($code -eq 3) {
  Write-Host '  SKIP: no rows at all -- Everything not installed/running; not counted as pass' -ForegroundColor Yellow
} else {
  $fail++
  Write-Host "  FAIL (exit $code): unexpected search failure" -ForegroundColor Red
  Write-Host (([string]$text).Trim())
}

# run-command: the search-box command source must have an equivalent scriptable
# entry so agents can exercise it without the GUI.
Write-Host '==> run-command: dry-run resolves a shell'
$out = Check 'run-command: dry-run' @('cli', 'run-command', 'echo efto-verify', '--dry-run')
if ($out -notmatch 'powershell|pwsh|cmd|bash|wsl') {
  $fail++; Write-Host '  FAIL: dry-run output does not look like a shell command' -ForegroundColor Red
}
Write-Host '==> run-command: echo round-trip'
$out = Check 'run-command: echo' @('cli', 'run-command', 'echo efto-verify')
if ($out -notmatch 'efto-verify') {
  $fail++; Write-Host '  FAIL: stdout missing the echo marker' -ForegroundColor Red
}

# capabilities: the machine-readable contract an agent reads before acting. If the
# advertised exit codes or the dry-run surface drift, agents break silently.
Write-Host '==> capabilities --json'
$f = Join-Path $work 'caps.txt'
$code = Run @('cli', 'capabilities', '--json') $f
$text = ReadOut $f
if ($code -eq 0 -and $text -match '"not_found": 3' -and $text -match '"--dry-run"') {
  Write-Host '  OK (subcommands, exit codes and dry-run surface advertised)' -ForegroundColor Green
} else {
  $fail++
  Write-Host "  FAIL (exit $code)" -ForegroundColor Red
  Write-Host (([string]$text).Trim())
}

# --dry-run must report without executing: this file must stay unopened.
Write-Host '==> files --dry-run'
$probe = Join-Path $work 'probe.txt'
Set-Content -LiteralPath $probe -Value 'probe' -Encoding ASCII
$f = Join-Path $work 'files.txt'
$code = Run @('cli', 'files', ('"' + $probe + '"'), '--dry-run', '--json') $f
$text = ReadOut $f
if ($code -eq 0 -and $text -match '"dry_run":true') {
  Write-Host '  OK (reported without opening)' -ForegroundColor Green
} else {
  $fail++
  Write-Host "  FAIL (exit $code)" -ForegroundColor Red
  Write-Host (([string]$text).Trim())
}

# A PID that cannot exist must be "not found" (3), never a silent success.
Check 'focus: unknown pid is not-found' @('cli', 'focus', '--pid', '999999', '--dry-run') $true

Write-Host '==> launch + cleanup (starts Notepad, stops that exact PID)'
# Start-Process with file redirection instead of cmd /c:
#   - cmd re-encodes CJK arguments through the ANSI code page and can mangle them;
#   - the launched GUI child then does not inherit our console/pipe handles,
#     so this script cannot be blocked by it.
$outFile = Join-Path $work 'launch.txt'
$errFile = Join-Path $work 'launch.err.txt'
$prev = $env:RUST_LOG
$env:RUST_LOG = 'info'
$proc = Start-Process -FilePath $exe -ArgumentList @('cli', 'launch', $jsb) `
  -NoNewWindow -PassThru -RedirectStandardOutput $outFile -RedirectStandardError $errFile
$null = $proc.WaitForExit(30000)
$code = if ($proc.HasExited) { $proc.ExitCode } else { -1 }
$env:RUST_LOG = $prev
$text = ''
if (Test-Path -LiteralPath $outFile) { $text = [string](Get-Content -LiteralPath $outFile -Raw) }
# tracing goes to stderr now (stdout must stay clean for --json); pid log lands there
$errText = ''
if (Test-Path -LiteralPath $errFile) { $errText = [string](Get-Content -LiteralPath $errFile -Raw) }
Write-Host ([string]$text).Trim()
$m = [regex]::Match($text + "`n" + $errText, 'pid=(\d+)')
if ($m.Success) {
  $targetPid = [int]$m.Groups[1].Value
  Start-Sleep -Seconds 1
  $alive = Get-Process -Id $targetPid -ErrorAction SilentlyContinue
  if ($alive) { Write-Host "  child alive: $($alive.ProcessName) (pid $targetPid)" -ForegroundColor Green }
  Stop-Process -Id $targetPid -Force -ErrorAction SilentlyContinue
  Write-Host "  stopped pid=$targetPid (cleanup done)" -ForegroundColor Green
  Write-Host '  OK' -ForegroundColor Green
} elseif ($code -eq 0) {
  Write-Host '  launched via ShellExecute (no pid in log); close that window manually' -ForegroundColor Yellow
} else {
  $fail++
  Write-Host "  FAIL (exit $code)" -ForegroundColor Red
}

Write-Host ''
if ($fail -eq 0) { Write-Host 'ALL CHECKS PASSED' -ForegroundColor Green }
else { Write-Host "$fail CHECK(S) FAILED" -ForegroundColor Red }
exit $fail
