# Next live batch: customer 053–075

Recommended cohort: **23 disks**, including previously partial **059 and 062**. Use a fresh project so this measures new physical reads, not reuse of an old completed scan. Numbered customer labels are authoritative; do not substitute the separate disposable test disks.

Use the current release directly, avoiding an old installed `fv` copy. Neither recipe needs a policy file, profile map or manual extraction. Automatic format detection, recovery, packed captures, background extraction/conversion and reports remain enabled. Four conversion workers are the conservative default. Deleted recovery remains OFF.

Choose **one** mode below. Dual mode tests throughput/custody/transfers across a larger cohort; GW-only is the simpler fallback if the USB station is unavailable. No unattended speed or whole-collection yield promise is implied.

## Both drives: recommended larger dual pilot

Prepare **customer 053 in USB A:** and **customer 054 in the working Mitsumi/GW selector B**, with both write-protect holes open. These are different disks; never share a disk between active readers. The USB option retains the shop's previously verified hardware-blocker assertion and current Windows checks.

Paste in PowerShell:

```powershell
$fv = 'C:\Users\User\Desktop\randomprojectsillneverfinish\FluxVault\target\release\fluxvault.exe'
$batch = 'C:\Users\User\Desktop\FluxVault-Test\Dual-053-075-' + [guid]::NewGuid().ToString('N')
& $fv init $batch
if ($LASTEXITCODE -ne 0) { throw 'Project creation failed; stop here.' }
Set-Location -LiteralPath $batch
& $fv disk select 53
if ($LASTEXITCODE -ne 0) { throw 'Initial disk selection failed; stop here.' }
& $fv scan --double --last-disk 75 --write-blocker-verified
```

At the aggregator prompt:

1. Type **`u53`** to start USB, then **`g54`** to start GW while USB reads.
2. Follow **NEXT FRESH** for the next unused label; type `u55` or `g55` into whichever station is free. Labels are shared, not fixed odd/even assignments. Never move a READING disk.
3. A USB partial says **SET ASIDE FOR GW**. When GW is free, insert that same numbered disk there and type its number with `g`, e.g. **`g59`**. This is a recovery transfer, not a new fresh label. USB can continue fresh feeding while GW recovers it. Only transfer identities the displayed queue makes available.
4. **Green SAVED** and **red PARTIAL SAVED** both allow removal. Red FAILED asks for same-label attention, not the next disk. If No Index requests a reseat, follow its same-label confirmation after activity stops.
5. At the end, finish pending USB-to-GW transfers. Remove each station's last saved disk and type **`u out`** / **`g out`**. Background work drains; wait for the final summary.

`s` shows status, `p` pauses new feeding, `r` resumes feeding, `q` stops new reads and drains active/background work. Empty Enter does not read in dual mode; `--no-verify` is intentionally unavailable here. A pending transfer or held saved disk can prevent final completion until resolved/removed.

### Resume this dual project

Keep the printed folder path. Reopen the same project and repeat the dual command, following saved custody rather than starting again at 053:

```powershell
$fv = 'C:\Users\User\Desktop\randomprojectsillneverfinish\FluxVault\target\release\fluxvault.exe'
Set-Location -LiteralPath 'C:\paste\the\printed\Dual-053-075-project'
& $fv scan --double --last-disk 75 --write-blocker-verified
```

Do not `init` or `disk select` on resume. A forced console exit is now covered by [Windows host supervision](PROCESS_SUPERVISION.md), but use graceful `q` for an ordinary break. Wait for physical activity to settle after abrupt exit and reconfirm the actual disk. This live pilot is not an instruction to crash a physical read deliberately.

## GW only: minimal typing

Use this **instead of** the dual recipe. Insert **customer 053** in the working Mitsumi/GW B, hole open:

```powershell
$pilot = 'C:\Users\User\Desktop\randomprojectsillneverfinish\FluxVault\scripts\start-pilot.ps1'
& $pilot -FirstDisk 53 -LastDisk 75 -NoVerify
```

It creates a fresh project and prints its path; no `cd` is needed because the launcher passes that project explicitly. Check the displayed label/protection before each **Enter**. Follow swap cues, ending at 075. `QUIT` stops early and drains saved work. Red partial banners still permit swapping. The launcher collects reports and prints **FLUXVAULT TEST SUMMARY** automatically.

Resume using the **same printed path/range**, never another fresh project:

```powershell
& $pilot -FirstDisk 53 -LastDisk 75 -NoVerify -Project 'C:\paste\the\printed\project'
```

## Collect after the dual run

Stay in its project folder. These commands use saved artifacts only; no floppy is needed:

```powershell
& $fv production benchmark
& $fv processing status
& $fv recovery queue
& $fv conversion issues
& $fv benchmark compare --baseline 'C:\Users\User\Desktop\randomprojectsillneverfinish\FluxVault\TextilMuzeum_Floppy_Archive_20260920_115210.zip'
```

Comparison scopes reference files to acquired labels. Exit **3** means attention/partial/missing, not necessarily a crash; **2** is an operation/input error. Do not reset a cursor or delete evidence to work around it. Finalized archival packages are separate from proving complete customer recovery; partial attention must stay explicit.

Send back the **project path** and **final console summary**, plus any confusing cue/failure/long quiet period. Detailed DualScan JSON, durable benchmark events/exports, acquisitions, conversion logs, partial maps and baseline snapshots are already local; no huge ZIP upload is needed. Measure reader overlap and elapsed wall time from those saved events, not the sum of concurrent read times. A completed live cohort supplies useful real throughput/recovery data, not automatic certification of all 136 disks.
