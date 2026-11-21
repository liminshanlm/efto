# Run a cargo command inside the MSVC dev environment.
# Pass the whole cargo command as ONE string: PowerShell treats a bare -- as its own separator.
#   powershell -File scripts/cargo.ps1 -Cmd "test -p efto-core"
#   powershell -File scripts/cargo.ps1 -Cmd "check --workspace"
param([string]$Cmd = "check --workspace")
. (Join-Path $PSScriptRoot 'vsenv.ps1')
$argv = $Cmd -split '\s+' | Where-Object { $_ -ne '' }
Write-Host ("cargo " + ($argv -join ' '))
& cargo @argv
exit $LASTEXITCODE
