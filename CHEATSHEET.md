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

### Optional: Tab completion (PowerShell 7)

```powershell
. 'C:\Users\User\Desktop\randomprojectsillneverfinish\FluxVault\scripts\FluxVault.Completion.ps1'
```

Then `fv sc` + Tab completes `scan`; `fv scan --no` + Tab completes `--no-verify`. Current terminal only; no drive probes or automatic profile edits. [Guide](SHELL_COMPLETION.md).

### New batch: create, enter, scan

Choose a **fresh folder name**. Do not initialize an existing archive to resume it.

```powershell
fv init 'C:\Users\User\Desktop\FluxVault-Test\My-New-Batch'
Set-Location 'C:\Users\User\Desktop\FluxVault-Test\My-New-Batch'
fv production start --last-disk 20 --no-verify
```

Check the label and open write-protect hole, insert the displayed disk, then press Enter. Omit `--no-verify` to type its number instead (`001` or `1`). `QUIT` stops at a waiting prompt. Never remove a disk while reading.

No policy/format file is needed. New scans identify supported 720 KB/1.44 MB formats, recover within limits, process saved files in the background and pack captures.

Production also builds a verified ZIP automatically at the checked endpoint, under sibling `My-New-Batch-Delivery`. Partial/raw-only results stay attention, not customer-certified. Early QUIT never archives an unfinished batch. [Guide](PRODUCTION_WORKFLOW.md).

Optional audible swap/error reminders: add `--sound on`, for example `fv scan --last-disk 136 --no-verify --sound on`. Off by default and not remembered on restart. USB/GW tones differ; clean/partial/error patterns differ. Dual USB partials use a separate rising transfer-to-GW pattern. Audio is best-effort and interactive-only: always follow the written saved/swap cue, not a sound alone. [Details](CLI.md#optional-scan-sound-cues).

Optional unpacked delivery copy: `fv package build --destination C:\CustomerPackages --keep-staging` keeps the exact verified ZIP members in a separate `.staging` folder beside the ZIP. Extra storage/time; your project stays unchanged. Automatic production remains ZIP-only.

### Existing batch: enter, inspect, continue

```powershell
Set-Location 'C:\Users\User\Desktop\FluxVault-Test\My-New-Batch'
fv status
fv production resume --no-verify
```

Insert the **displayed next/pending disk**, not necessarily 001. Repeat original settings/endpoint; do not run `init` or reset numbering. `--no-verify` skips label typing only and must be supplied each session.

### Quick visibility / optional notes

```powershell
fv status
fv disk show 59 --details
fv disk note 59 "Original label checked"
```

Status shows recorded stations/transfers, processing backlog and rough historical fresh-feed pace/ETA. The original scan window's current swap cue remains authoritative. Disk details include recovery/extraction/conversion/audit state; old reports are labelled historical. Notes are optional and do not renumber a disk or touch media; save them when the project is not busy. [Details](CLI.md#see-what-is-happening-without-interrupting-it).

### Stop now; continue later

Type `STOP` during a scan, or use Windows Ctrl+C. `QUIT` instead finishes reads and drains saved work.

```powershell
# Second console, while the first is busy:
fv stop --project 'C:\Users\User\Desktop\FluxVault-Test\My-New-Batch'
fv run status --project 'C:\Users\User\Desktop\FluxVault-Test\My-New-Batch'

# Resume the SAME production project (saved range/options):
fv production resume --no-verify
```

Keep disks seated until **STOPPED** and drive activity has stopped. Never reinitialize/reset to resume. [Full guide](STOP_RESUME.md).

Old/expert `scan` batches keep their original behavior; repeat that command. Its alias `start` is not the same as the full-chain `production start`/`resume`.

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

Destination must exist outside the project, never on a floppy. Default finalize blocks attention; add `--allow-attention` to explicitly archive partial results with their warnings (still exit 3). Use `fv finalize status` and `fv finalize resume` after a stop/crash; saved products are rechecked offline. Ctrl+C or second-console `fv stop` stops finishing. [Guide](FINALIZATION.md). Keep the project; a ZIP is not proof of complete recovery.

### Optional: bring in an old script ZIP

Choose a new folder whose parent exists; **do not run init first**:

```powershell
fv project import --source 'C:\archives\old-script-archive.zip' --destination 'C:\archives\Imported' --plan
fv project import --source 'C:\archives\old-script-archive.zip' --destination 'C:\archives\Imported'
Set-Location 'C:\archives\Imported'
Get-Content .\Reports\LegacyImport.txt | Out-Host -Paging
```

Exit 3 means successfully copied legacy evidence with attention. Source bytes/logs/short images stay intact; old files are not newly certified. Existing destinations are refused; interrupted staging is retained, not resumed. No floppy/tools needed. [Guide](LEGACY_IMPORT.md).

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

Exit codes: **0** completed · **3** attention/partial · **2** input/operation error · **130** operator stop. Red partial-saved is not failed. The bar shows activity/reported track visits, not recovered-sector yield.

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
| Next set-aside USB disk recommended for GW (dual scan) | `fv production queue` |
| Process saved images through full chain | `fv process` |
| Request 12 Office workers offline | `fv process --conversion-workers 12` |
| Native partial-file recovery | `fv recovery extract 59` |
| Forensic text from an incomplete Word file | `fv recovery documents 24` |
| Deleted candidates, forensic-only | `fv recovery extract 59 --include-deleted` |
| Extract eligible saved disks | `fv extract all` |
| Refresh inventory | `fv files manifest` |
| Mirror originals/plan conversions | `fv conversion plan` |
| Run Office conversions | `fv conversion run` |
| Retry saved conversion issues | `fv conversion retry` |
| Refresh full final report/inventories | `fv report export` |
| English final report | `fv report export --language en` |
| Identify saved raw format | `fv greaseweazle identify 9` |
| Pack capture, keep raw | `fv storage pack 7` |
| Pack, explicitly retire verified raw | `fv storage pack 7 --retire-raw` |

Normal scans/processing need their saved-file tools; standalone native extraction needs neither 7-Zip nor LibreOffice. Deleted candidates and raw fragments are not complete/live customer documents.

Final reports are automatic after saved-file processing. Open the printed `FloppyFinalReport.xlsx`; `Reports/FinalReportLatest.json` records the latest complete bundle. Standalone export needs no floppy/tools and changes no payload. [Statuses and limits](FINAL_REPORTS.md).

Native generation 5 / Word-text engine 2 automatically adds evidenced lost-directory recovery, bounded missing-FAT-link tail alternatives and Word text salvage, including supported containers with unrelated directory damage. No new scan flag is needed. `fv recovery documents 24` prints a readable offline `.html` edition path: open it to see the text with explicit gaps, without assembling segments yourself. Text/editions live under `Recovery`, not as repaired DOCs; the command returns attention code 3. [Simple commands and limits](DEEP_RECOVERY.md).

`fv process` automatically uses supported verified composite/FAT improvements for extraction and delivery. These appear as **DERIVED**, not clean physical reads; even zero-gap results retain attention. No manual copying from `Recovery` is needed. [Details](OFFLINE_RECOVERY_HANDOFF.md).

### Settings and diagnostics

Optional defaults for this project (no need to repeat the worker flag):

```powershell
fv project settings set conversion-workers 12
fv project settings set operator 'Archive team'
fv project settings
```

Advanced saved-image choice: `fv disk prefer 7 2` pins image attempt 002 for disk 007; `fv disk prefer 7 auto` restores quality ranking. Native extraction/inventory/audit refresh automatically; `fv process` reconciles Office outputs. Earlier evidence stays. [Tool overrides and limits](PROJECT_SETTINGS.md).

USB-only scanning now also runs saved-file processing in the background. Default: one forward pass, then set partials aside for GW. `--last-disk N` persists the endpoint; `--count N` only caps this invocation. `OUT` confirms final removal; switching to dual retains the transfer queue. Use `--acquisition-only` explicitly to skip processing. Never run competing scans in the same project.

`fv diagnose 59` inspects saved raw/packed captures and decode passes, replays final sector provenance and exports a short recovery note plus track/sector CSV and JSON. No floppy or host tool needed; exit `3` means attention/partial. [How to read it](FLUX_DIAGNOSTICS.md).

USB-only: `fv scan --usb --count 20 --write-blocker-verified` (default A:, same protection checks; type the number). Two-drive **pilot**: `fv scan --double --write-blocker-verified --last-disk 10`, then `u7` / `g8` for USB/GW labels, `gOLD` for a queued USB partial, `u out` / `g out` after the last saved disks, `QUIT` to drain. Use a small new project first; dual rejects `--no-verify`. Ordinary scan stays GW-only. [Copyable 007–010 test](DUAL_SCAN.md).

Dual-only break commands: `p` pauses new reads (current reads and files finish); `r` resumes confirmations, starting no read itself. The pause survives restart. Always wait for **SAVED** before removal, even while paused. `s` shows each station's next physical action and pending USB transfers.

Dual timing: `fv production benchmark` exports saved timing JSON/CSV offline. New scans collect this automatically and show a `PACE` line with a rough fresh-feed ETA after three fresh saves. Recovery transfers/file tail are extra; restart retains logs but resets the live sample. `fv benchmark report` is for single-GW sessions. [Details](DUAL_SCAN.md#pace-and-saved-timing-reports).

- `fv tools check` checks host tools; `fv greaseweazle info` checks board readiness.
- `fv scan --conversion-workers 4` saves a 1–16 worker ceiling. Shared CPU/RAM/storage admission can reduce active jobs to leave room for reads. `fv processing status` shows recorded waits/budget; no extra policy is needed.
- Fast/Normal/Recovery/Detective each get up to ten minutes of capture time. Clean/no-improvement stops finish earlier; stubborn disks can take roughly 40 minutes plus offline work.
- `--project 'C:\full\project\path'` selects a project without changing folder.
- `--json` gives machine-readable output; progress stays on stderr.
- `--color never` removes colored scan cues; text instructions remain.
- `fv --help` lists implemented commands. There is no GUI; two-drive mode remains an opt-in pilot, not full-collection production acceptance.
- Expert saved-byte check: `fv recovery sector 59 --lba 24 --sectors 2`. Add `--attempt 2` for an image attempt or `--json`; this never reads a floppy. Missing/derived bytes stay labeled. [Sector inspector](SECTOR_INSPECTION.md).

Selection/cleanup are automatic: verified richer generations become preferred, earlier evidence stays intact, and eligible obsolete original copies move into recoverable `Recovery/DeliveryQuarantine`. Edited/untracked/pre-ledger copies and old Office derivatives stay preserved. [Details](RECOVERY_SELECTION.md).

More: [CLI reference](CLI.md) · [policies](POLICIES.md) · [background work](BACKGROUND_PROCESSING.md) · [136-disk runbook](PILOT_136.md) · [archive comparison](BASELINE_COMPARISON.md).
