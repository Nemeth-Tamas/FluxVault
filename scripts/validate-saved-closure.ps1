# Repeatable saved-evidence acceptance. Never probes/reads physical drives.
[CmdletBinding()]
param(
    [string]$Project = 'C:\Users\User\Desktop\FluxVault-Test\Customer-136-Dual-20261010',
    [string]$CompressionProject = 'C:\Users\User\Desktop\FluxVault-Test\Customer-020-Pilot-20261006',
    [string]$Baseline = (Join-Path (Split-Path -Parent $PSScriptRoot) 'TextilMuzeum_Floppy_Archive_20260920_115210.zip'),
    [string]$Executable = (Join-Path (Split-Path -Parent $PSScriptRoot) 'target\release\fluxvault.exe'),
    [string]$OutputParent = 'C:\Users\User\Desktop\FluxVault-Test'
)
$ErrorActionPreference='Stop'
$Project=(Resolve-Path -LiteralPath $Project).Path
$CompressionProject=(Resolve-Path -LiteralPath $CompressionProject).Path
$Baseline=(Resolve-Path -LiteralPath $Baseline).Path
$Executable=(Resolve-Path -LiteralPath $Executable).Path
$OutputParent=(Resolve-Path -LiteralPath $OutputParent).Path
foreach($path in @($Project,$CompressionProject,$Baseline,$Executable,$OutputParent)) {if($path -match '^(?i)([AB]:|\\\\[.?]\\|\\Device\\)'){throw 'Use workstation paths, not media/devices.'}}
$receipt=Join-Path $OutputParent ('Saved-Closure-'+(Get-Date -Format yyyyMMdd)+'-'+[guid]::NewGuid().ToString('N').Substring(0,8))
New-Item -ItemType Directory -Path $receipt|Out-Null
# Stable customer source/handoff bytes; exclude mutable lock/worker telemetry.
$protected=@()
foreach($root in @($Project,$CompressionProject)) {
    foreach($folder in @('Images','Flux')) {
        $protected += Get-ChildItem -LiteralPath (Join-Path $root $folder) -Recurse -File | Where-Object {$_.Name -notlike '.*' -and $_.Name -notlike '*.partial*'}
    }
    $protected += Get-ChildItem -LiteralPath $root -File | Where-Object {$_.Name -eq 'project.json' -or $_.Name -eq 'DELIVERY_READY.json' -or $_.Name -like 'FINAL_*' -or $_.Name -like '*.zip' -or $_.Name -like '*.sha256'}
}
$protected += Get-Item -LiteralPath $Baseline
$hashes=@($protected|Sort-Object FullName -Unique|ForEach-Object{[pscustomobject]@{Path=$_.FullName;SHA256=(Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash}})
$hashes|Export-Csv -LiteralPath (Join-Path $receipt 'ProtectedInputs.csv') -NoTypeInformation -Encoding utf8
function Invoke-Saved([string]$Name,[string[]]$Arguments) {
    $text=& $Executable @Arguments 2> (Join-Path $receipt "$Name.stderr.log")
    $code=$LASTEXITCODE
    if($code -notin @(0,3)){throw "$Name failed with $code; logs retained in $receipt"}
    $joined=$text -join "`n"
    $joined|Set-Content -LiteralPath (Join-Path $receipt "$Name.json") -Encoding utf8
    $value=$joined|ConvertFrom-Json
    Write-Output -NoEnumerate $value
}
$legacy=Invoke-Saved 'LegacyInventory' @('benchmark','legacy','--baseline',$Baseline,'--json')
if($legacy.source_records -ne 1667 -or $legacy.extracted_members -ne 1667 -or $legacy.conversion_rows -ne 972 -or $legacy.discrepancies.Count -ne 0){throw 'Legacy inventory differs from the expected exact archive.'}
$dual=Invoke-Saved 'ProductionBenchmark' @('production','benchmark','--project',$Project,'--json')
if($dual.benchmark.verified_saved_unique_labels -ne 136 -or $dual.benchmark.operator_actions.disks.Count -ne 136){throw 'All 136 identities must be represented; missing observations stay unknown.'}
$estimate=Invoke-Saved 'ProductionEstimate' @('production','estimate','--project',$Project,'--json')
if($estimate.physical_media_access -ne $false){throw 'Forecast must be offline.'}
foreach($case in @(@{Disk=7;Attempt=1;Role='clean_full'},@{Disk=5;Attempt=1;Role='damaged_full'},@{Disk=5;Attempt=2;Role='targeted'})) {
    $b=Invoke-Saved "Compression-$($case.Role)" @('storage','benchmark',"$($case.Disk)",'--capture-attempt',"$($case.Attempt)",'--project',$CompressionProject,'--json')
    if($b.capture_attempt -ne $case.Attempt -or @($b.measurements|Where-Object{-not $_.roundtrip_verified -or $_.roundtrip_sha256 -ne $b.source_sha256}).Count){throw 'Compression must roundtrip the exact named source.'}
}
foreach($disk in @(10,17,22,133)) {
    $range=Invoke-Saved "Ranges-$disk" @('recovery','ranges',"$disk",'--project',$Project,'--json')
    if($range.project_modified -ne $false -or $range.physical_media_access -ne $false){throw 'Range inspection must be read-only.'}
    if($range.source_sha256 -notmatch '^[a-f0-9]{64}$'){throw 'Range report must bind an actual selected image hash.'}
}
# One actual upstream offline conversion, in a new isolated project only.
$expert=Join-Path $receipt 'ExpertOffline'
& $Executable init $expert|Out-Null
if($LASTEXITCODE -ne 0){throw 'Expert destination init failed.'}
$sourceCapture=Join-Path $CompressionProject 'Flux\009_attempt_001.scp'
$defs=Join-Path $PSScriptRoot 'fixtures\expert-dd.cfg'
$result=Invoke-Saved 'ExpertOffline' @('greaseweazle','expert','decode','9','--format','fv.customer.dd','--diskdefs',$defs,'--input',$sourceCapture,'--expected-bytes','737280','--project',$expert,'--json')
if($result.physical_media_access -ne $false -or $result.automatic_donor_eligible -ne $false -or $result.output_bytes -ne 737280){throw 'Expert result must retain unverified offline isolation.'}
$expected=Get-Content -LiteralPath (Join-Path $CompressionProject 'Images\009_attempt_001.json') -Raw|ConvertFrom-Json
if($result.output_sha256 -ne $expected.sha256){throw 'Actual offline DD expert decode differs from the saved selected image.'}
$half=Invoke-Saved 'Expert360Offline' @('greaseweazle','expert','decode','9','--format','ibm.360','--cylinders','0-39','--input',$sourceCapture,'--expected-bytes','368640','--project',$expert,'--json')
$imagePath=if([IO.Path]::IsPathRooted($expected.image_file)){$expected.image_file}else{Join-Path $CompressionProject ('Images\'+$expected.image_file)}
$sourceBytes=[IO.File]::ReadAllBytes($imagePath)
$halfHash=[Convert]::ToHexString([Security.Cryptography.SHA256]::HashData([byte[]]$sourceBytes[0..368639])).ToLowerInvariant()
if($half.output_sha256 -ne $halfHash -or $half.automatic_donor_eligible -ne $false){throw 'Additional builtin format interpretation must equal the observed source prefix and stay unverified.'}
# Compare like archive-content categories, not an uncompressed work folder to ZIP.
# The file populations differ, so these are accounting totals, not yield parity
# or an identical-content compression comparison; raw sample ratios above are exact.
$newZip=(Get-ChildItem -LiteralPath $Project -Filter '*.zip' -File | Sort-Object LastWriteTimeUtc -Descending | Select-Object -First 1).FullName
$classes=@()
foreach($archivePath in @($Baseline,$newZip)) {
    $z=[IO.Compression.ZipFile]::OpenRead($archivePath)
    try {
        foreach($category in @('Images','Flux','Extracted','Converted','Recovery','Reports','Logs')) {
            $members=@($z.Entries|Where-Object {$_.FullName.StartsWith($category+'/') -and -not $_.FullName.EndsWith('/')})
            $classes += [pscustomobject]@{Archive=$archivePath;Category=$category;Files=$members.Count;UncompressedBytes=($members|Measure-Object Length -Sum).Sum;ArchivedBytes=($members|Measure-Object CompressedLength -Sum).Sum;Scope='Corresponding categories; content populations differ. Not an equal-file yield/compression comparison.'}
        }
    } finally {$z.Dispose()}
}
$classes|ConvertTo-Json -Depth 4|Set-Content -LiteralPath (Join-Path $receipt 'ArchiveStorageClasses.json') -Encoding utf8
foreach($file in $hashes){if((Get-FileHash -LiteralPath $file.Path -Algorithm SHA256).Hash -ne $file.SHA256){throw "Protected original changed: $($file.Path)"}}
[pscustomobject]@{SchemaVersion=1;ExecutableSHA256=(Get-FileHash -LiteralPath $Executable -Algorithm SHA256).Hash;ProtectedFilesUnchanged=$hashes.Count;All136OperatorRows=$dual.benchmark.operator_actions.disks.Count;ConfirmedActions=$dual.benchmark.operator_actions.confirmed_operator_actions;LegacySourceRecords=$legacy.source_records;LegacyConversionJobs=$legacy.conversion_rows;PhysicalMediaAccess=$false;ExpertDDMatchesSavedImage=$true;Scope='Saved-evidence replay; no physical write-blocking or new hardware-performance certification.'}|ConvertTo-Json|Set-Content -LiteralPath (Join-Path $receipt 'Acceptance.json') -Encoding utf8
Write-Output "Saved closure accepted: $($hashes.Count) protected files unchanged. Receipt: $receipt"
