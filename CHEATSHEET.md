# FluxVault cheat sheet

**PowerShell · Windows · `fv` = `fluxvault`**

New here? Follow [the tutorial](TUTORIAL.md). This is the quick reference after setup. Physical scanning defaults to the tested Greaseweazle/Mitsumi station on selector B. USB commands are in the advanced reference.

## Beginner: everyday commands

### Install or refresh once

```powershell
Set-Location 'C:\Users\User\Desktop\randomprojectsillneverfinish\FluxVault'
powershell -NoProfile -File .\scripts\install-cli.ps1 -AddToPath
```

Reopen PowerShell, then `fv --help`. Reinstall after a new build to refresh the copied executable. No installation wanted? Use the full executable path with `& $fv`; [example](TUTORIAL.md#1-install-or-refresh-the-short-command).

### New batch: create, enter, scan

Choose a **fresh folder name**. Do not initialize an existing archive to resume it.

```powershell
fv init 'C:\Users\User\Desktop\FluxVault-Test\My-New-Batch'
Set-Location 'C:\Users\User\Desktop\FluxVault-Test\My-New-Batch'
fv scan --last-disk 20 --no-verify
```

Check the label and open write-protect hole, insert the displayed disk, then press Enter. Omit `--no-verify` to type its number instead (`001` or `1`). `QUIT` stops at a waiting prompt. Never remove a disk while reading.

No policy/format file is needed. New scans identify supported 720 KB/1.44 MB formats, recover within limits, process saved files in the background and pack captures.

### Existing batch: enter, inspect, continue

```powershell
Set-Location 'C:\Users\User\Desktop\FluxVault-Test\My-New-Batch'
fv status
fv scan --last-disk 20 --no-verify
```

Insert the **displayed next/pending disk**, not necessarily 001. Repeat original settings/endpoint; do not run `init` or reset numbering. `--no-verify` skips label typing only and must be supplied each session.

### After scanning

```powershell
fv status
fv recovery queue
fv conversion issues
fv benchmark report
```

Saved work interrupted? No floppy needed:

```powershell
fv processing resume
fv storage resume
```

### Delivery, when checks allow

```powershell
fv audit
New-Item -ItemType Directory -Path 'C:\Users\User\Desktop\FluxVault-Delivery' -Force
fv finalize --destination 'C:\Users\User\Desktop\FluxVault-Delivery'
```

Destination must exist outside the project. Finalize blocks automatic packaging if unresolved attention remains. Keep the project; a ZIP is not proof of complete recovery.

## Beginner: read the banner

| Cue | Meaning |
| --- | --- |
| Cyan **WAITING FOR YOU** | Insert/check/confirm displayed disk. |
| **READING / DO NOT REMOVE** | Wait; recovery stages may run automatically. |
| Green **DONE / REMOVE / INSERT** | Saved; swap. |
| Red **PARTIAL SAVED / REMOVE / INSERT** | Saved with unresolved sectors; still swap. |
| Amber **RAW-ONLY FORMAT EXCEPTION SAVED** | Raw preserved, no compatible image claimed; follow swap cue. |
| Red **REMOVE AND REINSERT SAME DISK** | Reseat same disk and reconfirm. |
| Red **FAILED** | Keep the same number/project; inspect error. |
| **BATCH FINISHED / REMOVE** | Remove last disk; no further insertion. |

Exit codes: **0** completed · **3** attention/partial · **2** input/operation error. Red partial-saved is not failed. The bar shows activity/reported track visits, not recovered-sector yield.

## Advanced: optional shortcuts

### Start at 053, end at 064

Select the start **before the first scan in a new project**:

```powershell
fv disk select 53
fv scan --last-disk 64 --no-verify
```

Resume with the scan line only. `--last-disk` is an absolute endpoint; `--count N` caps this invocation. Completed jobs are reused, not reread; use [the fresh-test launcher](TOMORROW_TEST.md) for repeat-read measurements.

### Saved-file tools: no physical disk needed

| Task | Command |
| --- | --- |
| Inspect attempts/LBAs/hashes | `fv disk show 59 --details` |
| Background progress | `fv processing status` |
| Process saved images through full chain | `fv process` |
| Request 12 Office workers offline | `fv process --conversion-workers 12` |
| Native partial-file recovery | `fv recovery extract 59` |
| Deleted candidates, forensic-only | `fv recovery extract 59 --include-deleted` |
| Extract eligible saved disks | `fv extract all` |
| Refresh inventory | `fv files manifest` |
| Mirror originals/plan conversions | `fv conversion plan` |
| Run Office conversions | `fv conversion run` |
| Retry saved conversion issues | `fv conversion retry` |
| Export workbook | `fv report export` |
| Identify saved raw format | `fv greaseweazle identify 9` |
| Pack capture, keep raw | `fv storage pack 7` |
| Pack, explicitly retire verified raw | `fv storage pack 7 --retire-raw` |

Normal scans/processing need their saved-file tools; standalone native extraction needs neither 7-Zip nor LibreOffice. Deleted candidates and raw fragments are not complete/live customer documents.

`fv process` automatically uses supported verified composite/FAT improvements for extraction and delivery. These appear as **DERIVED**, not clean physical reads; even zero-gap results retain attention. No manual copying from `Recovery` is needed. [Details](OFFLINE_RECOVERY_HANDOFF.md).

### Settings and diagnostics

USB-only: `fv scan --usb --count 20 --write-blocker-verified` (default A:, same protection checks; type the number). Two-drive **pilot**: `fv scan --double --write-blocker-verified --last-disk 10`, then `u7` / `g8` for USB/GW labels, `gOLD` for a queued USB partial, `u out` / `g out` after the last saved disks, `QUIT` to drain. Use a small new project first; dual rejects `--no-verify`. Ordinary scan stays GW-only. [Copyable 007–010 test](DUAL_SCAN.md).

Dual-only break commands: `p` pauses new reads (current reads and files finish); `r` resumes confirmations, starting no read itself. The pause survives restart. Always wait for **SAVED** before removal, even while paused. `s` shows each station's next physical action and pending USB transfers.

Dual timing: `fv production benchmark` exports saved timing JSON/CSV offline. New scans collect this automatically and show a `PACE` line with a rough fresh-feed ETA after three fresh saves. Recovery transfers/file tail are extra; restart retains logs but resets the live sample. `fv benchmark report` is for single-GW sessions. [Details](DUAL_SCAN.md#pace-and-saved-timing-reports).

- `fv tools check` checks host tools; `fv greaseweazle info` checks board readiness.
- `fv scan --conversion-workers 4` saves a 1–16 worker request; background work caps it to leave two logical CPUs available.
- Fast/Normal/Recovery/Detective each get up to ten minutes of capture time. Clean/no-improvement stops finish earlier; stubborn disks can take roughly 40 minutes plus offline work.
- `--project 'C:\full\project\path'` selects a project without changing folder.
- `--json` gives machine-readable output; progress stays on stderr.
- `--color never` removes colored scan cues; text instructions remain.
- `fv --help` lists implemented commands. There is no GUI; two-drive mode remains an opt-in pilot, not full-collection production acceptance.

Selection/cleanup are automatic: verified richer generations become preferred, earlier evidence stays intact, and eligible obsolete original copies move into recoverable `Recovery/DeliveryQuarantine`. Edited/untracked/pre-ledger copies and old Office derivatives stay preserved. [Details](RECOVERY_SELECTION.md).

More: [CLI reference](CLI.md) · [policies](POLICIES.md) · [background work](BACKGROUND_PROCESSING.md) · [136-disk runbook](PILOT_136.md) · [archive comparison](BASELINE_COMPARISON.md).
