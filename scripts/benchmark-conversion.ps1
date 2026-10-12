# Saved files only. Fresh isolated destinations: never reconverts customer outputs.
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Project,
    [string]$Executable = (Join-Path (Split-Path -Parent $PSScriptRoot) 'target\debug\fluxvault.exe'),
    [string]$OutputParent = 'C:\Users\User\Desktop\FluxVault-Test',
    [ValidateRange(6,60)][int]$FileCount = 24,
    [int[]]$Workers = @(4,8,12)
)
$ErrorActionPreference = 'Stop'
if (@($Workers | Where-Object { $_ -lt 1 -or $_ -gt 16 }).Count -or @($Workers | Select-Object -Unique).Count -ne $Workers.Count) { throw 'Use distinct worker counts 1..16.' }
$Project = (Resolve-Path -LiteralPath $Project).Path
$Executable = (Resolve-Path -LiteralPath $Executable).Path
$OutputParent = (Resolve-Path -LiteralPath $OutputParent).Path
foreach ($path in @($Project,$Executable,$OutputParent)) {
    if ($path -match '^(?i)([AB]:|\\\\[.?]\\|\\Device\\)') { throw 'Workstation paths only.' }
}
$snapshot = Get-Content -LiteralPath (Join-Path $Project 'Reports\ConversionState.json') -Raw | ConvertFrom-Json
$all = @($snapshot.result.planning.jobs | ForEach-Object {
    $job = $_
    $file = Get-Item -LiteralPath $job.source_path
    if ($file.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'Redirected source refused.' }
    if ($file.Length -gt 16MB) { throw 'Source exceeds sample size bound.' }
    $actual = (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actual -ne $job.source_sha256) { throw "Changed source: $($file.FullName)" }
    [pscustomobject]@{Path=$file.FullName;Bytes=$file.Length;SHA256=$actual;Extension=$file.Extension}
} | Sort-Object Bytes,Path -Unique)
if ($all.Count -lt $FileCount) { throw 'Not enough distinct saved conversion candidates.' }
# Evenly span small through large files rather than benchmarking only tiny jobs.
$sample = @(for ($i=0; $i -lt $FileCount; $i++) { $all[[int][Math]::Floor($i*($all.Count-1)/($FileCount-1))] })
$receipt = Join-Path $OutputParent ('Conversion-Workers-' + (Get-Date -Format yyyyMMdd) + '-' + [guid]::NewGuid().ToString('N').Substring(0,8))
New-Item -ItemType Directory -Path $receipt | Out-Null
$sample | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $receipt 'Samples.json') -Encoding utf8
$runs = @()
foreach ($workerCount in $Workers) {
    $trial = Join-Path $receipt "workers-$workerCount"
    & $Executable init $trial | Out-Null
    if ($LASTEXITCODE -ne 0) { throw 'Fresh trial init failed.' }
    $sourceDir = Join-Path $trial 'Extracted\001'
    New-Item -ItemType Directory -Path $sourceDir | Out-Null
    for ($i=0; $i -lt $sample.Count; $i++) {
        Copy-Item -LiteralPath $sample[$i].Path -Destination (Join-Path $sourceDir ('sample-{0:D3}{1}' -f $i,$sample[$i].Extension))
    }
    $startInfo = [Diagnostics.ProcessStartInfo]::new($Executable)
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    foreach ($a in @('conversion','run','--project',$trial,'--conversion-workers',"$workerCount",'--json')) { $startInfo.ArgumentList.Add($a) }
    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $startInfo
    $clock = [Diagnostics.Stopwatch]::StartNew()
    [void]$process.Start()
    $stdout = $process.StandardOutput.ReadToEndAsync()
    $stderr = $process.StandardError.ReadToEndAsync()
    $known = @{}; $peak = 0L; $observations = 0
    $known[$process.Id] = 0.0
    while (-not $process.HasExited) {
        if ($clock.Elapsed.TotalMinutes -gt 10) { $process.Kill($true); throw 'Owned benchmark process timed out; isolated trial preserved.' }
        # Follow parent relationships, not all LibreOffice processes on the machine.
        $inventory = @(Get-CimInstance Win32_Process | Select-Object ProcessId,ParentProcessId)
        do {
            $changed = $false
            foreach ($entry in $inventory) {
                if ($known.ContainsKey([int]$entry.ParentProcessId) -and -not $known.ContainsKey([int]$entry.ProcessId)) { $known[[int]$entry.ProcessId]=0.0; $changed=$true }
            }
        } while ($changed)
        $resident = 0L
        foreach ($processId in @($known.Keys)) {
            try {
                $p = Get-Process -Id $processId -ErrorAction Stop
                $resident += $p.WorkingSet64
                $known[$processId] = [Math]::Max($known[$processId],$p.TotalProcessorTime.TotalSeconds)
            } catch [Microsoft.PowerShell.Commands.ProcessCommandException] { }
        }
        $peak = [Math]::Max($peak,$resident)
        $observations++
        Start-Sleep -Milliseconds 100
        $process.Refresh()
    }
    $process.WaitForExit(); $clock.Stop()
    $outText=$stdout.GetAwaiter().GetResult(); $errText=$stderr.GetAwaiter().GetResult()
    $outText | Set-Content -LiteralPath (Join-Path $trial 'benchmark-output.json') -Encoding utf8
    $errText | Set-Content -LiteralPath (Join-Path $trial 'benchmark-progress.log') -Encoding utf8
    if ($process.ExitCode -notin @(0,3)) { throw "Conversion failed with code $($process.ExitCode); logs preserved in $trial" }
    $result = $outText | ConvertFrom-Json
    $state = Get-Content -LiteralPath (Join-Path $trial 'Reports\ConversionState.json') -Raw | ConvertFrom-Json
    $valid=0; $invalid=0
    foreach ($row in $state.result.rows) {
        foreach ($kind in @('modern','pdf')) {
            $path = if ($kind -eq 'modern') { $row.job.modern_path } else { $row.job.pdf_path }
            $expected = $row.$kind.output_sha256
            if (-not $expected) { $invalid++; continue }
            $actual=(Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()
            if ($actual -ne $expected) { throw 'Generated output hash mismatch.' }
            if ($kind -eq 'modern') {
                $zip=[IO.Compression.ZipFile]::OpenRead($path)
                try {
                    if (-not $zip.GetEntry('[Content_Types].xml')) { throw 'OOXML required member missing.' }
                    foreach ($entry in $zip.Entries) { $stream=$entry.Open(); try {$stream.CopyTo([IO.Stream]::Null)} finally {$stream.Dispose()} }
                } finally { $zip.Dispose() }
            } else {
                $f=[IO.File]::OpenRead($path); try { $header=[byte[]]::new(5); if ($f.Read($header,0,5) -ne 5 -or [Text.Encoding]::ASCII.GetString($header) -ne '%PDF-') {throw 'PDF header invalid.'} } finally {$f.Dispose()}
            }
            $valid++
        }
    }
    $runs += [pscustomobject]@{Workers=$workerCount;Candidates=$result.candidates;OK=$result.ok;Partial=$result.partial;Failed=$result.failed;Timeouts=$result.timed_out;ReusedOutputs=$result.reused_outputs;ElapsedSeconds=$clock.Elapsed.TotalSeconds;JobsPerMinute=60*$result.ok/$clock.Elapsed.TotalSeconds;SampledTreePeakResidentBytes=$peak;ObservedTreeCPUSeconds=($known.Values | Measure-Object -Sum).Sum;ResourceObservations=$observations;OutputsHashAndContainerChecked=$valid;MissingOutputs=$invalid;Project=$trial}
    $process.Dispose()
    $runs | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $receipt 'Results.json') -Encoding utf8
    Write-Output ("{0} workers: {1} OK / {2} timeouts / {3:N1}s / {4:N0} MiB sampled process-tree peak" -f $workerCount,$result.ok,$result.timed_out,$clock.Elapsed.TotalSeconds,($peak/1MB))
}
foreach ($file in $sample) { if ((Get-FileHash -LiteralPath $file.Path -Algorithm SHA256).Hash.ToLowerInvariant() -ne $file.SHA256) { throw 'Customer source changed during benchmark.' } }
[pscustomobject]@{SchemaVersion=1;SampleFiles=$sample.Count;SourceProject=$Project;SourceUnchanged=$true;PhysicalMediaAccess=$false;Runs=$runs;Scope='Single workstation trial per worker count; sampled current-process-tree RSS/CPU are lower-bound observations (short-lived children may be missed); no hard peak/linear speedup guarantee. Fresh isolated saved-file conversions, not a physical scan.'} | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $receipt 'Acceptance.json') -Encoding utf8
Write-Output "Performance receipt: $receipt"
