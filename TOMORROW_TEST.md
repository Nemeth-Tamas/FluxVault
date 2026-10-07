# Tomorrow: two short checks, ready-to-review results

Use the freshly rebuilt executable through this launcher. No installation, policy file or format map required. Each invocation without `-Project` creates a **new** isolated project under `Desktop\FluxVault-Test` and prints its path.

## 1. Customer 009: DD smoke test

Insert **customer 009** in the working Mitsumi/Greaseweazle drive (selector B), write-protect hole open:

```powershell
$pilot = 'C:\Users\User\Desktop\randomprojectsillneverfinish\FluxVault\scripts\start-pilot.ps1'
& $pilot -FirstDisk 9 -LastDisk 9 -NoVerify
```

Check the displayed label, press Enter, and wait for the swap/finished banner before removing the disk. The bar should stay animated while reading and disappear before the next instruction. The expected supported geometry is 720 KB / 1,440 sectors. A clean read does not mean the historical carved Word/JPEG candidates have been recovered; that gap remains explicit in the comparison.

## 2. Customer 058: investigate the reference difference

Insert **customer 058**, with the protection hole open:

```powershell
& $pilot -FirstDisk 58 -LastDisk 58 -NoVerify
```

Its last scan had a clean sector image but several document-byte differences versus the old DMDE archive. Some archived versions are deleted/ambiguous drafts. A fresh, isolated repeat helps separate reproducible captured content from historical archive differences. This command does not reconstruct or overwrite customer files with guesses.

## Optional next cohort

If both checks behave well and you have time, feed 021–032:

```powershell
& $pilot -FirstDisk 21 -LastDisk 32 -NoVerify
```

Default policies, format discovery, packed captures and background processing remain enabled. A red **PARTIAL SAVED** banner permits swapping; a red **FAILED** message does not advance custody. Amber raw-only format exceptions also permit swapping, but are not complete extracted disks. `QUIT` ends feeding and drains saved-file work. The launcher collects results even after an ordinary scan error when the saved project can be inspected.

## Send back

Paste the final **FLUXVAULT TEST SUMMARY** and tell me:

1. Did the reading bar animate and clear before the instruction?
2. Were the green/red swap cues obvious at a glance?
3. Any error text, unexpected label or long quiet delay? Stop and paste the error; do not change disk selection to work around it.

The launcher writes unique `Reports/TestSummary-*.txt/.json`, benchmark snapshots and the baseline file comparison. I can inspect detailed local logs from the printed project path; no need to upload customer files or a huge ZIP.

## Resume or collect without hardware

**Current 021–032 cohort stopped at 022 (2026-10-07).** The saved 021 is complete with seven matching baseline payloads; 022's first attempt returned No Index and remains pending. After updating to the reseat-prompt build, remove/reinsert **022**, check the open protection hole, and resume this exact project:

```powershell
$pilot = 'C:\Users\User\Desktop\randomprojectsillneverfinish\FluxVault\scripts\start-pilot.ps1'
& $pilot -FirstDisk 21 -LastDisk 32 -NoVerify -Project 'C:\Users\User\Desktop\FluxVault-Test\Customer-021-032-Test-59ed47d3e9214caba22730cfa88e291e'
```

It continues at 022, not 021. A subsequent No Index capture failure produces a red same-disk reseat/reconfirm prompt rather than exiting immediately; two retries are allowed per disk/invocation. Failed metadata/partial captures stay saved. Never advance the physical label until the saved-result swap banner asks for the next disk.

Resume **the same** printed project/range after interruption; never `init` it again or reset its disk cursor:

```powershell
& $pilot -FirstDisk 21 -LastDisk 32 -NoVerify -Project 'C:\paste\the\printed\project'
```

To regenerate a results summary without scanning, with no floppy needed:

```powershell
& $pilot -CollectOnly -Project 'C:\paste\the\printed\project'
```

Deleted-file recovery stays **off**. Confirmed DMDE deleted payloads are excluded from ordinary baseline scoring. The explicit comparison-only `--include-deleted` flag and uncertainty rules are explained in [BASELINE_COMPARISON.md](BASELINE_COMPARISON.md).

Avoid other mutating commands while a scan owns the project. `fv processing status --project 'C:\the\project'` in another terminal is safe. The launcher preserves existing projects, keeps all previous snapshots and never resets the overall test workspace.
