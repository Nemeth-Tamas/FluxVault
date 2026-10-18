# Build a portable operator package. Never collects projects/captures or changes PATH.
[CmdletBinding()]
param([Parameter(Mandatory = $true)][string]$Destination)
$ErrorActionPreference = 'Stop'
$repoRoot = [IO.Path]::GetFullPath((Split-Path -Parent $PSScriptRoot))
$outputRoot = [IO.Path]::GetFullPath($Destination)
if ($outputRoot -ieq $repoRoot -or $outputRoot.StartsWith($repoRoot + '\', [StringComparison]::OrdinalIgnoreCase)) {
    throw 'Choose a release destination outside the source checkout.'
}
Push-Location -LiteralPath $repoRoot
try {
    & cargo build --locked --release --bin fluxvault
    if ($LASTEXITCODE -ne 0) { throw 'Release build failed.' }
    $exe = Join-Path $repoRoot 'target\release\fluxvault.exe'
    $version = (& $exe --version).Trim() -replace '^FluxVault ', ''
    if ($LASTEXITCODE -ne 0 -or $version -notmatch '^\d+\.\d+\.\d+$') { throw 'Invalid executable version.' }
    $name = "FluxVault-$version-windows-x64"
    $stage = Join-Path $outputRoot $name
    $archive = Join-Path $outputRoot "$name.zip"
    if ((Test-Path -LiteralPath $stage) -or (Test-Path -LiteralPath $archive) -or (Test-Path -LiteralPath "$archive.sha256")) {
        throw 'Release destination already exists. Preserve it; choose a new output folder.'
    }
    New-Item -ItemType Directory -Path (Join-Path $stage 'scripts') -Force | Out-Null
    Copy-Item -LiteralPath $exe -Destination (Join-Path $stage 'fluxvault.exe')
    Copy-Item -LiteralPath $exe -Destination (Join-Path $stage 'fv.exe')
    foreach ($file in @('install-cli.ps1','FluxVault.Completion.ps1')) {
        Copy-Item -LiteralPath (Join-Path $PSScriptRoot $file) -Destination (Join-Path $stage "scripts\$file")
    }
    Copy-Item -LiteralPath (Join-Path $repoRoot 'FLUXVAULT-MANUAL.txt') -Destination $stage
    foreach ($file in @('README.md','TODO.md')) {
        Copy-Item -LiteralPath (Join-Path $repoRoot $file) -Destination $stage
    }
    Copy-Item -LiteralPath (Join-Path $repoRoot 'docs\guides\V1_RELEASE.md') -Destination (Join-Path $stage 'START_HERE.md')
    Copy-Item -LiteralPath (Join-Path $repoRoot 'docs') -Destination $stage -Recurse
    $inventory = @(Get-ChildItem -LiteralPath $stage -Recurse -File | Sort-Object FullName | ForEach-Object {
        '{0}  {1}' -f (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant(), $_.FullName.Substring($stage.Length + 1).Replace('\','/')
    })
    $inventory | Set-Content -LiteralPath (Join-Path $stage 'SHA256SUMS.txt') -Encoding utf8
    Compress-Archive -LiteralPath $stage -DestinationPath $archive -CompressionLevel Optimal
    $sha = (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant()
    "$sha  $name.zip" | Set-Content -LiteralPath "$archive.sha256" -Encoding utf8
    Write-Output "Portable release: $archive"
    Write-Output "SHA-256: $sha"
} finally { Pop-Location }
