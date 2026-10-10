# Developer gate only. Never installs tools, starts a production scan, or changes
# user PATH/profile. Environment-dependent hardware/real-tool tests stay opt-in.
[CmdletBinding()]
param([switch]$SkipTests)
$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path -Parent $PSScriptRoot
Push-Location -LiteralPath $repoRoot
try {
    $checks = @(
        @('fmt', '--check'),
        @('check', '--locked', '--all-targets'),
        @('clippy', '--locked', '--all-targets', '--', '-D', 'warnings')
    )
    if (-not $SkipTests) { $checks += ,@('test', '--locked', '--all-targets') }
    foreach ($arguments in $checks) {
        Write-Host ("Quality gate: cargo {0}" -f ($arguments -join ' '))
        & cargo @arguments
        if ($LASTEXITCODE -ne 0) { throw "Quality gate failed with exit $LASTEXITCODE; later checks were not run." }
    }
    if ($SkipTests) { Write-Warning 'Tests were explicitly skipped; this is not a complete verification gate.' }
    else { Write-Output 'All FluxVault quality gates passed.' }
} finally { Pop-Location }
