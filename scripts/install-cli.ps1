[CmdletBinding(SupportsShouldProcess = $true)]
param(
    [string]$InstallDir = (Join-Path $env:LOCALAPPDATA 'Programs\FluxVault'),
    [switch]$AddToPath
)

$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path -Parent $PSScriptRoot
$source = Join-Path $repoRoot 'target\release\fluxvault.exe'
if (-not (Test-Path -LiteralPath $source -PathType Leaf)) {
    throw "Release executable not found: $source. Run cargo build --release first."
}

$resolvedInstall = [IO.Path]::GetFullPath($InstallDir)
$destination = Join-Path $resolvedInstall 'fluxvault.exe'
if ($PSCmdlet.ShouldProcess($destination, 'Install FluxVault CLI executable')) {
    New-Item -ItemType Directory -Path $resolvedInstall -Force | Out-Null
    Copy-Item -LiteralPath $source -Destination $destination -Force
}

if ($AddToPath) {
    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    $entries = @($userPath -split ';' | Where-Object { $_ -ne '' })
    $alreadyPresent = $false
    foreach ($entry in $entries) {
        $expanded = [Environment]::ExpandEnvironmentVariables($entry)
        try {
            if ([IO.Path]::GetFullPath($expanded).TrimEnd('\') -ieq $resolvedInstall.TrimEnd('\')) {
                $alreadyPresent = $true
                break
            }
        } catch {
            # Ignore unrelated malformed PATH entries; do not remove them.
        }
    }
    if (-not $alreadyPresent -and $PSCmdlet.ShouldProcess('User PATH', "Append $resolvedInstall")) {
        [Environment]::SetEnvironmentVariable('Path', (($entries + $resolvedInstall) -join ';'), 'User')
        Write-Output 'Open a new terminal for the updated user PATH to take effect.'
    }
}

Write-Output "FluxVault CLI target: $destination"
