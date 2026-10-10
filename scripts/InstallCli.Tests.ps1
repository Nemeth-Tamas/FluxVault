param([Parameter(Mandatory)][string]$FixtureRoot)
$ErrorActionPreference = 'Stop'
$root = [IO.Path]::GetFullPath($FixtureRoot)
if (-not (Test-Path -LiteralPath $root -PathType Container) -or
    -not ([IO.Path]::GetFileName($root).StartsWith('fv-install-fixture-'))) {
    throw 'Expected a pre-created isolated installation fixture.'
}
$install = Join-Path $root 'Installed CLI with spaces'
$project = Join-Path $root 'Project with spaces'
$beforeUserPath = [Environment]::GetEnvironmentVariable('Path', 'User')
$beforeProcessPath = $env:Path
& (Join-Path $PSScriptRoot 'install-cli.ps1') -InstallDir $install -WhatIf
if (Test-Path -LiteralPath $install) { throw 'WhatIf created installation files' }
& (Join-Path $PSScriptRoot 'install-cli.ps1') -InstallDir $install
$source = Join-Path (Split-Path -Parent $PSScriptRoot) 'target\release\fluxvault.exe'
$expected = (Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash
foreach ($name in @('fluxvault.exe','fv.exe')) {
    if ((Get-FileHash -LiteralPath (Join-Path $install $name) -Algorithm SHA256).Hash -ne $expected) {
        throw 'Installed aliases differ from release executable'
    }
}
if ([Environment]::GetEnvironmentVariable('Path', 'User') -cne $beforeUserPath -or $env:Path -cne $beforeProcessPath) {
    throw 'Installer changed PATH without opt-in'
}
# Only this test process and its children see this PATH. No registry/profile changes.
$env:Path = "$install;$beforeProcessPath"
try {
    if ((Get-Command fv -CommandType Application).Source -ine (Join-Path $install 'fv.exe')) {
        throw 'PowerShell did not resolve the installed short alias'
    }
    & fv init $project
    if ($LASTEXITCODE -ne 0) { throw 'Installed init failed' }
    Push-Location -LiteralPath $project
    try {
        $psStatus = (& fv status --json | Out-String) | ConvertFrom-Json
        if ($LASTEXITCODE -ne 0 -or $psStatus.disks -ne 0 -or $psStatus.operations.physical_media_access -ne $false) {
            throw 'Installed PowerShell status contract failed'
        }
        $cmdStatus = (& $env:ComSpec /d /c 'fv status --json' | Out-String) | ConvertFrom-Json
        if ($LASTEXITCODE -ne 0 -or $cmdStatus.name -cne $psStatus.name) { throw 'Installed CMD status failed' }
        $automation = [Diagnostics.ProcessStartInfo]::new()
        $automation.FileName = Join-Path $install 'fluxvault.exe'
        $automation.WorkingDirectory = $project
        $automation.ArgumentList.Add('project')
        $automation.ArgumentList.Add('show')
        $automation.ArgumentList.Add('--json')
        $automation.UseShellExecute = $false
        $automation.RedirectStandardOutput = $true
        $automation.RedirectStandardError = $true
        $process = [Diagnostics.Process]::Start($automation)
        $json = $process.StandardOutput.ReadToEnd() | ConvertFrom-Json
        $errors = $process.StandardError.ReadToEnd()
        if (-not $process.WaitForExit(10000)) { $process.Kill($true); throw 'Automation smoke timed out' }
        if ($process.ExitCode -ne 0 -or $errors -or $json.current_disk -ne 1) { throw 'Installed automation failed' }
        $process.Dispose()
    } finally { Pop-Location }
} finally { $env:Path = $beforeProcessPath }
if ([Environment]::GetEnvironmentVariable('Path', 'User') -cne $beforeUserPath) { throw 'Persistent user PATH changed' }
Write-Output 'Isolated PowerShell / CMD / automation installation checks passed; user PATH unchanged.'
