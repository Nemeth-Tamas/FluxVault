# Fresh numbered test, or resume the exact printed project after interruption.
# -CollectOnly never starts a scan or invokes physical hardware.
[CmdletBinding()]
param(
    [ValidateRange(1, 9999)][int]$FirstDisk = 9,
    [ValidateRange(1, 9999)][int]$LastDisk = 9,
    [string]$Project,
    [string]$TestRoot = (Join-Path $env:USERPROFILE 'Desktop\FluxVault-Test'),
    [string]$Executable = (Join-Path $PSScriptRoot '..\target\release\fluxvault.exe'),
    [string]$Baseline = (Join-Path $PSScriptRoot '..\TextilMuzeum_Floppy_Archive_20260920_115210.zip'),
    [switch]$NoVerify,
    [switch]$CollectOnly
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
# Attention exit code 3 is a valid completed partial result, not a PS exception.
if (Get-Variable PSNativeCommandUseErrorActionPreference -ErrorAction SilentlyContinue) {
    $PSNativeCommandUseErrorActionPreference = $false
}
if ($LastDisk -lt $FirstDisk) { throw 'LastDisk must be at least FirstDisk.' }
if (!(Test-Path -LiteralPath $Executable -PathType Leaf)) {
    throw "Release executable missing: $Executable"
}
$Executable = (Resolve-Path -LiteralPath $Executable).Path
if ($CollectOnly -and !$Project) { throw '-CollectOnly requires an existing -Project.' }
if (!$Project) {
    $Project = Join-Path $TestRoot ('Customer-{0:D3}-{1:D3}-Test-{2}' -f $FirstDisk, $LastDisk, [guid]::NewGuid().ToString('N'))
}
$Project = [System.IO.Path]::GetFullPath($Project)
if ($Project -match '^(\\\\[?.]\\)?[AB]:') { throw 'Use a workstation project, never A: or B:.' }
$projectFile = Join-Path $Project 'project.json'
if (!(Test-Path -LiteralPath $projectFile -PathType Leaf)) {
    if ($CollectOnly) { throw "Existing project required: $Project" }
    # Refuse to initialize an existing directory: protect unrelated test material.
    if (Test-Path -LiteralPath $Project) { throw 'Choose a new directory or an existing FluxVault project.' }
    & $Executable init $Project
    if ($LASTEXITCODE -ne 0) { throw 'Project creation failed.' }
    & $Executable disk select $FirstDisk --project $Project
    if ($LASTEXITCODE -ne 0) { throw 'Initial disk selection failed.' }
}

function Get-PilotJson([string[]]$CommandArguments) {
    $text = & $Executable @CommandArguments --project $Project --json
    $code = $LASTEXITCODE
    if ($code -notin @(0, 3)) {
        throw "FluxVault exited $code during $($CommandArguments -join ' '): $text"
    }
    return ($text -join "`n" | ConvertFrom-Json)
}

$state = Get-PilotJson @('status')
if ($state.background_processing.owner_active) {
    throw 'The project is busy. Finish or stop its scan before collecting this test.'
}
if (!$CollectOnly -and ($state.current_disk -lt $FirstDisk -or $state.current_disk -gt ($LastDisk + 1))) {
    throw "Saved cursor $($state.current_disk) is outside this range. Resume with the original FirstDisk/LastDisk; do not reset selection."
}
Write-Host "`nTEST PROJECT: $Project" -ForegroundColor Cyan
Write-Host 'Resume with this same -Project path; omit -Project to create another fresh test.'
$scanExit = $null
if (!$CollectOnly) {
    Write-Host ('Insert the displayed numbered disk with its write-protect hole OPEN. Wait for GW SWAP before removing it.') -ForegroundColor Cyan
    $scanArguments = @('scan', '--last-disk', "$LastDisk", '--project', $Project)
    if ($NoVerify) { $scanArguments += '--no-verify' }
    & $Executable @scanArguments
    $scanExit = $LASTEXITCODE
    if ($scanExit -notin @(0, 3)) {
        Write-Warning "Scan stopped with exit $scanExit. Keep the project and resume with the same range and -Project. Collecting saved results below."
    }
}

$benchmark = Get-PilotJson @('benchmark', 'report')
$state = Get-PilotJson @('status')
$processing = Get-PilotJson @('processing', 'status')
$queue = Get-PilotJson @('recovery', 'queue')
$comparison = $null
if (Test-Path -LiteralPath $Baseline -PathType Leaf) {
    if ($state.disks -gt 0 -or $state.raw_format_exceptions.Count -gt 0) {
        $comparison = Get-PilotJson @('benchmark', 'compare', '--baseline', $Baseline)
    }
} else { Write-Warning 'Reference ZIP not found; baseline comparison skipped.' }

$notes = @(
    'FLUXVAULT TEST SUMMARY - paste this section back to the development chat',
    "Project: $Project",
    "Scan exit: $scanExit (blank = offline collection only; 3 = attention, not a crash)",
    "Images: $($state.disks); OK: $($state.ok_disks); partial: $($state.partial_disks); next: $($state.current_disk)",
    "Raw-only exceptions: $($state.raw_format_exceptions.Count)",
    "Benchmark unique disks: $($benchmark.benchmark.unique_committed_disks); recovery errors: $($benchmark.benchmark.recovery_errors); downstream errors: $($benchmark.benchmark.downstream_errors)",
    "Benchmark JSON: $($benchmark.summary)",
    'Deleted recovery is OFF. Confirmed deleted reference files are excluded; unknown/carved content remains visible.',
    'Also report: did the read bar animate/clear, and were green/red swap cues obvious?'
)
if ($comparison) {
    $c = $comparison.comparison
    $notes += "Baseline payloads: $($c.reference_payloads); identical: $($c.matched_payloads); changed: $($c.changed_payloads); missing: $($c.missing_payloads)"
    $notes += "Baseline JSON: $($comparison.summary)"
}
$reports = (Resolve-Path -LiteralPath (Join-Path $Project 'Reports')).Path
$resolvedProject = (Resolve-Path -LiteralPath $Project).Path
if ([System.IO.Directory]::GetParent($reports).FullName -ne $resolvedProject) {
    throw 'Reports directory escapes the project.'
}
$nonce = [guid]::NewGuid().ToString('N')
$summaryPath = Join-Path $reports "TestSummary-$nonce.txt"
$dataPath = Join-Path $reports "TestSummary-$nonce.json"
$data = [ordered]@{ project = $Project; scan_exit = $scanExit; collected_only = [bool]$CollectOnly; benchmark = $benchmark; status = $state; processing = $processing; recovery_queue = $queue; comparison = $comparison }
# Never overwrite an earlier result or any customer evidence.
foreach ($entry in @(@($summaryPath, ($notes -join "`r`n")), @($dataPath, ($data | ConvertTo-Json -Depth 80)))) {
    $stream = [System.IO.File]::Open($entry[0], [System.IO.FileMode]::CreateNew, [System.IO.FileAccess]::Write)
    try {
        $bytes = [System.Text.Encoding]::UTF8.GetBytes($entry[1])
        $stream.Write($bytes, 0, $bytes.Length)
        $stream.Flush($true)
    } finally { $stream.Dispose() }
}
Write-Host "`n============================================================" -ForegroundColor Cyan
Write-Host ($notes -join "`n")
Write-Host "Saved summary: $summaryPath" -ForegroundColor Cyan
Write-Host '============================================================' -ForegroundColor Cyan
