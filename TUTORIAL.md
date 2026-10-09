# FluxVault tutorial

No coding knowledge needed. Examples use **PowerShell on Windows**. Copy commands inside the blocks, not shell prompts such as `PS>` or `#`.

- [Beginner: first project and scan](#beginner-first-project-and-scan)
- [Beginner: stop, resume and results](#beginner-stop-resume-and-results)
- [Beginner: prepare delivery](#beginner-prepare-delivery)
- [Advanced: optional controls](#advanced-optional-controls)
- [Troubleshooting](#troubleshooting)

## Beginner: first project and scan

### 1. Install or refresh the short command

The current local executable is already built. Install the latest copy as `fv` and `fluxvault`:

```powershell
Set-Location 'C:\Users\User\Desktop\randomprojectsillneverfinish\FluxVault'
powershell -NoProfile -File .\scripts\install-cli.ps1 -AddToPath
```

Close and reopen PowerShell, then:

```powershell
fv --help
```

Run the installer again after a new build: installed commands are **copies**, not live links to the repository. If the executable is missing, run `cargo build --release` from the repository; this requires Rust, not editing code.

Prefer no installation? In each new PowerShell window:

```powershell
$fv = 'C:\Users\User\Desktop\randomprojectsillneverfinish\FluxVault\target\release\fluxvault.exe'
& $fv --help
```

Replace `fv` at the start of later examples with `& $fv`, for example `& $fv status`. The `&` runs the executable path stored in the variable.

### 2. Make a new project

A project is one folder containing images, files, reports and saved progress. It starts at disk 001. Choose a fresh name for another new batch.

```powershell
fv init 'C:\Users\User\Desktop\FluxVault-Test\My-New-Batch'
Set-Location 'C:\Users\User\Desktop\FluxVault-Test\My-New-Batch'
fv status
```

`Set-Location` and `cd` mean the same thing. Inside a project or its subfolders, FluxVault finds it automatically. To open an existing project, **only change folder**; do not run `init` again.

### 3. Check the setup

Connect the tested Greaseweazle/Mitsumi setup, then:

```powershell
fv tools check
fv greaseweazle info
```

`tools check` checks installed programs. `greaseweazle info` checks board availability, not floppy contents. Resolve missing-tool/board-not-ready messages before feeding disks. Normal scans also preflight processing tools before the first prompt.

The current straight ribbon uses **selector B**, already the new-scan default. The older NEC is not the validated drive. Other hardware needs [its own setup check](GREASEWEAZLE_PREFLIGHT.md).

### 4. Feed the disks

For 001 through 020:

```powershell
fv scan --last-disk 20
```

At each prompt:

1. Match the physical label to the displayed number.
2. Check the write-protect hole is open and insert the floppy.
3. Type its number (`001` or `1`) and press Enter.
4. Wait. Do not remove it while reading/retrying.
5. Follow **SAVED / REMOVE / INSERT**, then repeat.

The loading bar shows activity/reported tracks, not recovered-sector percentage. Recovery, conversion, reports and packing are automatic. You do not pick stages or make policy files.

For Enter-only confirmations:

```powershell
fv scan --last-disk 20 --no-verify
```

Check the physical label/protection, then press Enter after each swap. `--no-verify` skips **only typing the label**; hashes/evidence checks stay on. Add it each session; it is not saved.

### 5. Understand the banners

| Banner | Your action |
| --- | --- |
| Cyan **WAITING FOR YOU / INSERT** | Insert/check the displayed floppy, then confirm. |
| **READING / DO NOT REMOVE** | Wait; retries may run automatically. |
| Green **DONE / REMOVE / INSERT** | Saved; swap as instructed. |
| Red **PARTIAL SAVED / REMOVE / INSERT** | Saved with unresolved sectors; still swap. |
| Amber **RAW-ONLY FORMAT EXCEPTION SAVED** | Raw saved, no compatible image claimed; follow swap instruction. |
| Red **REMOVE AND REINSERT SAME DISK** | Reseat that same disk and reconfirm. |
| Red **FAILED** | Keep the same number/project; inspect the error. |
| **BATCH FINISHED / REMOVE** | Remove last disk. Do not insert the next cursor number. |

Read the text if colors are unavailable. Partial does not mean nothing was recovered: independently readable files and validated candidates can still be saved.

## Beginner: stop, resume and results

### Stop early

Type `QUIT` at a waiting prompt. Feeding stops and saved-file work finishes. Do not pull a floppy out during a physical read to stop the program.

To cancel active work instead, type `STOP`, or use Windows Ctrl+C. From another terminal: `fv stop --project 'C:\full\path\to\your\project'`. Wait for the original console's **STOPPED** cue **and drive activity to stop** before moving disks. `fv run status --project PATH` checks its owner offline. Cancellation exits `130`; pending labels, spent stage time and partial evidence remain resumable. `fv start` means `fv scan`, not reset. [Full guide](STOP_RESUME.md).

If the console unexpectedly closes, Windows stops FluxVault's supervised external host processes and their children. Keep partial files and reopen the same project/scan command; wait for drive activity to settle and reconfirm its displayed disk. Completed raw captures can resume offline decoding, but partial captures are not successful reads. This is crash cleanup, not a replacement for graceful `QUIT`. [Details and tested boundaries](PROCESS_SUPERVISION.md).

### Resume the same batch

Reopen PowerShell, enter the **same project**, repeat the scan command:

```powershell
Set-Location 'C:\Users\User\Desktop\FluxVault-Test\My-New-Batch'
fv status
fv scan --last-disk 20 --no-verify
```

Insert the **number displayed now**, not necessarily 001. Saved numbering/evidence is reconciled automatically. `--last-disk 20` stays the endpoint, not twenty more reads. Keep pending-job settings unchanged. Never reset the cursor or edit internal JSON files to work around an error.

A completed recovery is checked/reused, not reread. Use a **new project** for a genuinely fresh repeat-read benchmark; [the launcher](TOMORROW_TEST.md) does this for you.

### Check results

After the scan:

```powershell
fv status
fv disk list
fv recovery queue
fv conversion issues
```

During scanning, a **second terminal** can safely run `fv processing status --project 'C:\Users\User\Desktop\FluxVault-Test\My-New-Batch'`. Do not launch competing extraction/conversion/report writers in a busy project.

After interrupted saved-file work, no floppy needed:

```powershell
fv processing resume
fv storage resume
```

`fv process` also runs the saved-image recovery/extraction/conversion/audit/report chain. It never reads a physical disk.

Newer native recovery automatically looks for surviving lost directories, supported missing-link file-tail alternatives and readable text inside incomplete Word documents, including supported containers with unrelated damaged directory entries. You need not choose a strategy. To inspect one saved disk's Word-text outcome, run `fv recovery documents 24`. Open the printed `.html` edition path in your browser to read all surviving segments in order, with missing ranges clearly marked. Separate `.txt` segments and a gap/source report remain under `Recovery`; the edition is a generated forensic presentation, not a repaired DOC or extra complete original. No manual text assembly or Office repair step. **Exit 3 is normal attention.** [Beginner commands and advanced evidence](DEEP_RECOVERY.md).

When saved attempts can supply missing sectors or readable FAT redundancy can repair a FAT gap, processing automatically hands the verified improved image to file extraction and delivery. It appears as **DERIVED**, not a clean physical read; attention is still expected even with no remaining gaps. Earlier evidence and operator-created recovery folders are preserved. [Details and limits](OFFLINE_RECOVERY_HANDOFF.md).

### Find your files

The full report is automatic after processing. Refresh/open its location with `fv report export` (optionally `--language en`). Open the printed `FloppyFinalReport.xlsx`: disk status, recovered files, conversion issues, integrity and delivery hashes are together. No floppy/LibreOffice/Excel automation needed. [Report guide](FINAL_REPORTS.md).

| Folder | Contents |
| --- | --- |
| `Images` | Sector images, acquisition metadata and explicitly labeled DERIVED offline improvements. |
| `Flux` | Raw/packed captures and decode/format evidence. |
| `Extracted` | Forensic recovered originals and preserved generations. |
| `Converted` | Delivery-friendly originals and successful Office/PDF copies. |
| `Recovery` | Derived evidence, partial fragments, forensic Word text and optional deleted candidates. |
| `Reports` | Inventories, audit, workbook, benchmarks, selection/cleanup reports. |
| `Logs` | Acquisition and processing details. |

Keep the entire project. `.bin` fragments are not complete documents; carved names are reconstructed. Obsolete unchanged program-owned original mirrors may move into recoverable `Recovery/DeliveryQuarantine`; edited/untracked copies and earlier forensic generations remain preserved. Reports explain uncertainty and moves.

## Beginner: prepare delivery

After scanning/draining, inspect the queue and issues, then:

```powershell
fv audit
New-Item -ItemType Directory -Path 'C:\Users\User\Desktop\FluxVault-Delivery' -Force
fv finalize --destination 'C:\Users\User\Desktop\FluxVault-Delivery'
```

The destination must exist **outside the project**. `finalize` processes saved files and creates/verifies an archive only if checks allow it. Attention can block automatic packaging. Keep the original project regardless.

`package build` is an expert archival command, **not** a shortcut to recovery certification. An archive can preserve partial evidence without proving complete customer recovery.

## Advanced: optional controls

USB-only: `fv scan --usb --write-blocker-verified` uses A: and asks for each number. The **opt-in two-drive pilot** is `fv scan --double --write-blocker-verified`: one console, `u1` / `g2`, and `gOLD` for an earlier USB partial. Both readers feed background processing. Exact label/open-tab and existing USB protection checks remain; dual rejects `--no-verify`. Start with the [small 007–010 test](DUAL_SCAN.md), not 136 disks. Ordinary `fv scan` remains GW-only.

After a dual run, `fv production benchmark` exports the saved timing summary and receipt CSV without a floppy. New dual scans collect this automatically and show an approximate fresh-feed ETA after three fresh saves; remaining recovery transfers/file processing are separate. Older pilots have explicit timing gaps, not reconstructed measurements. [Details](DUAL_SCAN.md#pace-and-saved-timing-reports).

In dual mode, `p` pauses new reads while current reads/files finish; `r` enables numbered confirmations again but starts no read. Pause survives restarting the command. `s` shows both station actions and pending transfers. Only remove a disk after **SAVED**, even while paused; `u out` / `g out` records removal.

Ignore this section for a normal new scan. Replace example paths/numbers with your own as needed.

### Scan a numbered range

In a **new project before its first scan**:

```powershell
fv disk select 53
fv scan --last-disk 64 --no-verify
```

This reads 053 through 064. On resume, repeat the scan only; do not select 53 again. `--count 5` caps results finalized this invocation; it is not an absolute endpoint.

### Adjust workers and storage

```powershell
fv scan --last-disk 64 --conversion-workers 4
fv process --conversion-workers 12
```

Workers are parallel Office jobs, not floppy drives. Requests 1–16 are accepted. Background scanning caps them to leave two logical CPUs available; more may not be faster. Twelve can be requested for image-only processing on your machine. [Details](BACKGROUND_PROCESSING.md).

New scans pack verified complete captures automatically. `fv storage pack 7` packs saved evidence and keeps raw; `fv storage pack 7 --retire-raw` explicitly retires raw after verification. Containers preserve exact original bytes. `fv storage resume` finishes pending packing offline and safely checks abandoned temporary compression/decode copies after an interruption; active readers and unknown files are left alone. `--capture-storage raw` is an optional new-job scan choice. [Storage/restart details](CAPTURE_STORAGE.md).

### Recovery budgets and formats

The built-in Fast/Normal/Recovery/Detective stages each allow up to **ten minutes of capture time**. Clean/no-improvement stops finish earlier. A stubborn disk can use roughly 40 minutes plus offline work. [Custom policies are optional](POLICIES.md).

New scans discover supported IBM 720 KB/1.44 MB formats. `--profile ibm.720` or `--profile ibm.1440` pins a known format; maps handle known mixed batches. Expert single-disk `greaseweazle recover` has different defaults: explicitly use `--profile auto --gw-drive B` for automatic discovery on this station. Do not change pending-job format, policy, selector or endpoint.

### Saved-evidence tools

No inserted floppy needed:

```powershell
fv disk show 59 --details
fv recovery extract 59
fv recovery extract 59 --include-deleted
fv files manifest
fv conversion plan
fv conversion retry
fv benchmark report
```

Native extraction returns attention even after salvaging files. Deleted recovery is **off by default**, forensic-only and separate from normal delivery. Retry checks saved source bindings. Tool paths, capture/decode controls, guarded USB operations and JSON usage are in [CLI.md](CLI.md).

Compare against the old script/DMDE archive:

```powershell
fv benchmark compare --baseline 'C:\Users\User\Desktop\randomprojectsillneverfinish\FluxVault\TextilMuzeum_Floppy_Archive_20260920_115210.zip'
```

The ZIP is read, not modified. On **comparison**, `--include-deleted` changes scoring scope; it does not run deleted recovery. [Interpretation](BASELINE_COMPARISON.md).

## Troubleshooting

For a difficult **saved** disk, `fv diagnose 59` produces a short recovery note and detailed track/pass/sector data without reading it again. See [saved-flux diagnostics](FLUX_DIAGNOSTICS.md); this is optional investigation, not another required scan step.

| Symptom | Next step |
| --- | --- |
| `fv` not recognized | Reopen PowerShell after installation, or use `& $fv` with the executable path. |
| Installed command seems old | Run the installer again after the release build. |
| Project not found | Enter its folder or add `--project 'C:\full\project\path'`. |
| Missing processing tool | `fv tools check`; configure unusual paths using [CLI.md](CLI.md). |
| Board absent | Check power/USB/cabling, then `fv greaseweazle info`. |
| No Index prompt | Reseat the **same** disk and reconfirm; two guided retries. |
| Partial/exit `3` | Read queue/audit; not necessarily a crash. |
| Error/exit `2` | Keep project/error text; do not reset numbering or delete evidence. |
| Project busy | Wait for its owner; read-only status remains available. |
| Finalize blocked | Inspect recovery/conversion/audit attention. |

Keep logs and the exact project path when reporting a problem. Nothing uploads automatically. Do not enable source writes or disable protection to fix these errors.
