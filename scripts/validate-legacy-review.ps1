[CmdletBinding()]
param(
    [string]$Project='C:\Users\User\Desktop\FluxVault-Test\Customer-136-Dual-20261010',
    [string]$Pilot='C:\Users\User\Desktop\FluxVault-Test\Customer-020-Pilot-20261006',
    [string]$Baseline=(Join-Path (Split-Path -Parent $PSScriptRoot) 'TextilMuzeum_Floppy_Archive_20260920_115210.zip'),
    [string]$Executable=(Join-Path (Split-Path -Parent $PSScriptRoot) 'target\release\fluxvault.exe'),
    [string]$OutputParent='C:\Users\User\Desktop\FluxVault-Test'
)
$ErrorActionPreference='Stop'
foreach($value in @($Project,$Pilot,$Baseline,$Executable,$OutputParent)) {
    if($value -match '^(?i)([AB]:|\\\\[.?]\\|\\Device\\)'){throw 'Saved workstation evidence only.'}
}
$receipt=Join-Path $OutputParent ('Legacy-Review-'+(Get-Date -Format yyyyMMdd)+'-'+[guid]::NewGuid().ToString('N').Substring(0,8))
New-Item -ItemType Directory -Path $receipt|Out-Null
$protected=@((Get-Item -LiteralPath $Baseline))
foreach($root in @($Project,$Pilot)){
    $protected+=Get-ChildItem -LiteralPath (Join-Path $root 'Images') -File
    $protected+=Get-ChildItem -LiteralPath $root -File|Where-Object{$_.Name -eq 'project.json' -or $_.Name -eq 'DELIVERY_READY.json' -or $_.Name -like 'FINAL_*' -or $_.Name -like '*.zip' -or $_.Name -like '*.sha256'}
}
$seals=@($protected|ForEach-Object{[pscustomobject]@{Path=$_.FullName;SHA256=(Get-FileHash -LiteralPath $_.FullName).Hash}})
$seals|ConvertTo-Json -Depth 6|Set-Content -LiteralPath (Join-Path $receipt 'ProtectedBefore.json') -Encoding utf8
foreach($disk in @(9,17,20,58)){
    $source=if($disk -eq 58){$Project}else{$Pilot}
    $text=& $Executable benchmark review "$disk" --baseline $Baseline --project $source --json
    if($LASTEXITCODE -ne 3){throw "Review $disk must retain investigative attention."}
    $v=($text -join "`n")|ConvertFrom-Json
    if($v.physical_media_access -ne $false -or $v.host_tools_invoked -ne $false -or $v.project_modified -ne $false -or $v.archive_modified -ne $false){throw 'Review must stay read-only.'}
    $text|Set-Content -LiteralPath (Join-Path $receipt ('Review-'+$disk+'.json')) -Encoding utf8
}
foreach($seal in $seals){if((Get-FileHash -LiteralPath $seal.Path).Hash -ne $seal.SHA256){throw "Protected evidence changed: $($seal.Path)"}}
[pscustomobject]@{Schema=1;Disks=@(9,17,20,58);PhysicalMediaAccess=$false;ProtectedArtifactsUnchanged=$seals.Count;ExecutableSHA256=(Get-FileHash -LiteralPath $Executable).Hash;BaselineSHA256=(Get-FileHash -LiteralPath $Baseline).Hash;Limits='Classification, byte/stream comparisons and explicit unknown ownership; not new recovery or historical yield parity.'}|ConvertTo-Json -Depth 5|Set-Content -LiteralPath (Join-Path $receipt 'Acceptance.json') -Encoding utf8
Write-Host "Saved reference review: $receipt"
Get-Content -LiteralPath (Join-Path $receipt 'Acceptance.json')
