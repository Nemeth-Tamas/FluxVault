[CmdletBinding()]
param(
    [string]$Project='C:\Users\User\Desktop\FluxVault-Test\Customer-136-Dual-20261010',
    [string]$DdProject='C:\Users\User\Desktop\FluxVault-Test\Customer-020-Pilot-20261006',
    [string]$Executable=(Join-Path (Split-Path -Parent $PSScriptRoot) 'target\release\fluxvault.exe'),
    [string]$OutputParent='C:\Users\User\Desktop\FluxVault-Test'
)
$ErrorActionPreference='Stop'
$Project=(Resolve-Path -LiteralPath $Project).Path
$DdProject=(Resolve-Path -LiteralPath $DdProject).Path
$Executable=(Resolve-Path -LiteralPath $Executable).Path
$OutputParent=(Resolve-Path -LiteralPath $OutputParent).Path
foreach($path in @($Project,$DdProject,$Executable,$OutputParent)){if($path -match '^(?i)([AB]:|\\\\[.?]\\|\\Device\\)'){throw 'Workstation saved evidence only.'}}
$receipt=Join-Path $OutputParent ('Flux-CRC-'+(Get-Date -Format yyyyMMdd)+'-'+[guid]::NewGuid().ToString('N').Substring(0,8))
New-Item -ItemType Directory -Path $receipt|Out-Null
$protected=@()
foreach($root in @($Project,$DdProject)){
    foreach($folder in @('Images','Flux')){$protected+=Get-ChildItem -LiteralPath (Join-Path $root $folder) -Recurse -File|Where-Object{$_.Name -notlike '.*' -and $_.Name -notlike '*.partial*'}}
    $protected+=Get-ChildItem -LiteralPath $root -File|Where-Object{$_.Name -eq 'project.json' -or $_.Name -eq 'DELIVERY_READY.json' -or $_.Name -like 'FINAL_*' -or $_.Name -like '*.zip' -or $_.Name -like '*.sha256'}
}
$seals=@($protected|ForEach-Object{[pscustomobject]@{Path=$_.FullName;SHA256=(Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash}})
$seals|ConvertTo-Json -Depth 6|Set-Content -LiteralPath (Join-Path $receipt 'ProtectedBefore.json') -Encoding utf8
function Invoke-Crc([string]$name,[string[]]$arguments){
    $text=& $Executable @arguments
    $code=$LASTEXITCODE
    $text|Set-Content -LiteralPath (Join-Path $receipt ($name+'.json')) -Encoding utf8
    if($code -notin @(0,3)){throw "$name refused unexpectedly, exit $code"}
    $value=($text -join "`n")|ConvertFrom-Json
    if($value.physical_media_access -ne $false -or $value.project_modified -ne $false -or $value.host_tools_invoked -ne $false){throw 'CRC inspection must stay offline/read-only.'}
    return $value
}
$batch=Invoke-Crc 'Batch' @('recovery','crc-check','all','--project',$Project,'--json')
if($batch.disk_count -ne 136){throw 'Full cohort must retain all 136 identities.'}
$clean=Invoke-Crc 'Clean-HD-002' @('recovery','crc-check','2','--project',$Project,'--json')
if($clean.validated_lbas.Count -ne 2880 -or -not $clean.complete_sector_image){throw 'Known saved HD sample must validate all bytes.'}
$dd=Invoke-Crc 'Clean-DD-009' @('recovery','crc-check','9','--project',$DdProject,'--json')
if($dd.validated_lbas.Count -ne 1440 -or -not $dd.complete_sector_image){throw 'Known saved DD sample must validate all bytes.'}
foreach($disk in @(59,66,133)){
    $v=Invoke-Crc "Damaged-$disk" @('recovery','crc-check',"$disk",'--project',$Project,'--json')
    $accounted=$v.validated_lbas.Count+$v.unconfirmed_lbas.Count+$v.conflicting_crc_valid_lbas.Count+$v.unresolved_lbas.Count
    if($accounted -ne 2880){throw 'Every HD sector must have an explicit proof/unconfirmed/conflict/unresolved outcome.'}
    if($v.unresolved_lbas.Count -gt 0 -and $v.complete_sector_image){throw 'A partial map cannot be a complete image.'}
}
foreach($seal in $seals){if((Get-FileHash -LiteralPath $seal.Path -Algorithm SHA256).Hash -ne $seal.SHA256){throw "Protected source changed: $($seal.Path)"}}
[pscustomobject]@{
    Schema=1;PhysicalMediaAccess=$false;ProtectedArtifactsUnchanged=$seals.Count
    ExecutableSHA256=(Get-FileHash -LiteralPath $Executable -Algorithm SHA256).Hash
    BatchLabels=$batch.disk_count;CompleteCrcBytePasses=$batch.complete_passes;PartialCrcBytePasses=$batch.partial_passes
    AttentionOrRefused=$batch.attention_or_refused;NoCommittedFlux=$batch.not_applicable
    CleanHDValidated=$clean.validated_lbas.Count;CleanDDValidated=$dd.validated_lbas.Count
    Limits='CRC byte observations only, not physical label identity, semantic completeness, weak-bit certification or historical file-yield parity.'
}|ConvertTo-Json -Depth 8|Set-Content -LiteralPath (Join-Path $receipt 'Acceptance.json') -Encoding utf8
Write-Host "Saved CRC acceptance: $receipt"
Get-Content -LiteralPath (Join-Path $receipt 'Acceptance.json')
