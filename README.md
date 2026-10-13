# FluxVault

**Read floppies. Keep the evidence. Recover files. Prepare a checked customer archive.**

FluxVault is a Windows-first command-line floppy archiving and recovery tool. The everyday workflow is small: insert a protected, numbered floppy, confirm it, wait for the swap banner, and repeat. Saved-file recovery, conversion, reports and lossless capture packing run automatically in new scans.

There is no GUI. Both `fv` and `fluxvault` run the same program.

Saved damaged documents also get automatic content-based Word/RTF text salvage. `fv recovery documents 22` or `fv recovery documents 128` prints readable offline HTML and, where verified text survives, editable DOCX copies with explicit missing-text/symbol gaps. These are separate forensic derivatives, not repaired originals or inflated whole-file counts. [Examples and limits](docs/recovery/DEEP_RECOVERY.md).

Developing FluxVault? `pwsh -NoProfile -File .\scripts\check.ps1` is the fail-fast format/build/strict-lint/routine-test gate. It does not scan hardware, install tools or modify user PATH. Environment-dependent acceptance tests remain explicit opt-ins.

Current checklist: **463/479 TODOs (96.7%), 16 open**. [Latest offline checkpoint](docs/development/OFFLINE_CHECKPOINT_463.md), [current progress](docs/development/PROGRESS.md) and [the measured 136-disk review](docs/testing/BATCH_136_REVIEW.md) separate engineering completion, historical file differences and remaining recovery work. [Saved-batch tools](docs/guides/SAVED_BATCH_TOOLS.md) include completion scenarios, independent saved-flux CRC checks, legacy aggregate/candidate/stream comparisons, operator-action accounting and expert GW settings. Managed-file traces cover recovered files, conversions and forensic editable editions. The normal production command is unchanged. Checklist coverage is not a customer-file recovery rate; earlier checkpoints remain historical.

## Start here

The [documentation index](docs/README.md) groups operator guides, recovery/evidence, testing, development and historical handoffs. Only this overview and `TODO.md` remain in the root; the Windows text manual stays available there too.

| Your experience | Open this |
| --- | --- |
| New to FluxVault or PowerShell | [Beginner tutorial](docs/guides/TUTORIAL.md#beginner-first-project-and-scan) |
| Set up; just need commands | [Cheat sheet](docs/guides/CHEATSHEET.md) |
| Want optional settings and individual tools | [Advanced tutorial](docs/guides/TUTORIAL.md#advanced-optional-controls) and [CLI reference](docs/guides/CLI.md) |
| Running a measured batch | [136-disk runbook](docs/testing/PILOT_136.md) or [small-test launcher](docs/testing/TOMORROW_TEST.md) |
| Understand a difficult saved disk | [Saved-flux diagnostics](docs/recovery/FLUX_DIAGNOSTICS.md): `fv diagnose 59` |
| See which files depend on bad/disputed sectors | [Batch damage impact and file trace](docs/recovery/EVIDENCE_IMPACT.md): `fv recovery impact all` or `fv recovery impact 59` |
| Stop active work and continue later | [Stop/resume](docs/guides/STOP_RESUME.md): `STOP`, `fv stop`, `fv start` |
| Recovered files, conversion issues and delivery hashes | [Final report guide](docs/guides/FINAL_REPORTS.md): `fv report export` |
| Finish/archive saved results and resume interrupted finishing | [Finishing guide](docs/guides/FINALIZATION.md): `fv finalize --destination PATH` |
| Check benchmark readiness | [Single-GW gates](docs/testing/SINGLE_GW_READINESS.md) |
| Less typing in PowerShell 7 | [Tab completion](docs/guides/SHELL_COMPLETION.md): optional one-line setup |
| Want development status | [Progress](docs/development/PROGRESS.md) and [TODO](TODO.md) |
| Bring an original script ZIP into a fresh project | [Archive import](docs/guides/LEGACY_IMPORT.md): `fv project import --source ZIP --destination NEW_FOLDER` |

## Beginner: the usual workflow

These examples use **PowerShell**, with `fv` already installed. Choose a **new folder name** for a new project; never initialize/reset an existing archive to resume it.

```powershell
fv init 'C:\Users\User\Desktop\FluxVault-Test\My-New-Batch'
Set-Location 'C:\Users\User\Desktop\FluxVault-Test\My-New-Batch'
fv production start --last-disk 20
```

At each prompt, insert the displayed disk with its write-protect hole open, check its label, and type its number, such as `001`. No `READ` prefix is necessary. `QUIT` stops feeding and lets saved-file work finish.

Need to cancel an active read? Type `STOP`, use Windows Ctrl+C, or run `fv stop --project 'C:\full\project\path'` in another console. Wait for **STOPPED** and drive activity to stop before moving disks. Resume the same command/project; `fv start` is an alias for `fv scan`. [Details](docs/guides/STOP_RESUME.md). Exit `130` means operator cancellation.

For less typing, add `--no-verify` and press **Enter only after swapping and checking the label/protection**. This skips label typing, not evidence verification.

Resume from the **same project** with `fv production resume` (add `--no-verify`/`--sound on` again if wanted). The saved endpoint remains 020, not twenty more reads. Finishing is automatic: the verified ZIP/checksum and `FINAL_*` reports appear **beside `project.json`**. The original archive remains in sibling `My-New-Batch-Delivery`; `DELIVERY_READY.json` binds the ready copies to that snapshot. Partial results stay attention, never customer-certified. For an already-finished older batch, `fv package publish` adds this layout offline without rebuilding/converting/rescanning. [Production guide](docs/guides/PRODUCTION_WORKFLOW.md).

Existing `scan`/`start` retain their original behavior. Continue old batches with their original command; use a fresh project for the complete production path.

After scanning:

```powershell
fv status
fv recovery queue
fv conversion issues
```

The [tutorial](docs/guides/TUTORIAL.md) covers setup, damaged disks, interruptions and delivery step by step.

`fv status` now combines recorded stations, transfers, background work and rough fresh-feed pace. `fv disk show N --details` shows saved recovery/extraction/conversion/audit state, and `fv disk note N "TEXT"` saves an optional note without changing acquisition identity. Historical reports are labelled as such; the original console's current swap cue remains authoritative. [Inspection and notes](docs/guides/CLI.md#see-what-is-happening-without-interrupting-it).

Optional conveniences: `--sound on` adds station-specific swap/error reminders, including a distinct rising USB-to-GW transfer cue in dual mode. `fv package build --destination PATH --keep-staging` keeps a verified unpacked copy of the ZIP beside it; extra storage/time, with automatic production still ZIP-only. [CLI details](docs/guides/CLI.md).

## What you need

- Windows and a built FluxVault executable. [Installation](docs/guides/TUTORIAL.md#1-install-or-refresh-the-short-command) requires no coding when the executable is present.
- The official Greaseweazle host tool and a working board/drive for physical scans.
- 7-Zip and LibreOffice for normal automatic saved-file processing.
- Workstation storage for the project, never a floppy as the output destination.

The validated local station is the **Mitsumi drive, straight ribbon, Greaseweazle selector B**. New scans default to B. Other wiring may need another selector; see [hardware setup](docs/testing/GREASEWEAZLE_PREFLIGHT.md). Greaseweazle B is **not** Windows `B:`.

New scans use supported IBM 720 KB/1.44 MB automatic identification, background processing, packed captures, four requested conversion workers and built-in recovery limits. **No policy file or format list is required.** Expert single-disk commands have different defaults; see [CLI reference](docs/guides/CLI.md).

Background processing and capture packing automatically share CPU/RAM/storage admission so new reads take priority. No extra settings are needed. If processing waits, `fv processing status` shows its recorded stage and resource budget; [background processing](docs/guides/BACKGROUND_PROCESSING.md#automatic-resource-admission) explains pressure and safe resume.

## Read the result

- **DONE / REMOVE / INSERT**: saved; swap when instructed.
- **PARTIAL SAVED / REMOVE / INSERT**: saved with unresolved sectors; swapping is still allowed.
- **RAW-ONLY FORMAT EXCEPTION SAVED**: raw evidence saved, but no compatible sector image claimed; follow its swap instruction.
- **FAILED**: do not advance the physical label. Keep the project and read the error.

FluxVault never writes to source floppies. Missing bytes are not fabricated. Fragments, carved candidates and deleted-file evidence are labeled separately. A successful command or ZIP does not prove every original file was recovered. Exit `3` means attention/partial, not necessarily a crash; exit `2` means input/operation error.

## Advanced: deeper reading

| Topic | Guide |
| --- | --- |
| Optional recovery budgets | [Policies without homework](docs/guides/POLICIES.md) |
| Background work and conversion workers | [Background processing](docs/guides/BACKGROUND_PROCESSING.md) |
| Lossless capture packing and safe restart | [Capture storage](docs/guides/CAPTURE_STORAGE.md) |
| Console closes/crashes: host cleanup and restart | [Process supervision](docs/development/PROCESS_SUPERVISION.md) |
| Next measured 053–075 batch, single or dual drive | [Next scan](docs/testing/NEXT_SCAN.md) |
| Native recovery and missing boot metadata | [Damaged-filesystem recovery](docs/recovery/DAMAGED_FILESYSTEM_RECOVERY.md) |
| Automatic orphan/signature/embedded salvage | [Carving recovery](docs/recovery/CARVING_RECOVERY.md) |
| Fragments and opt-in deleted recovery | [Deleted and fragment recovery](docs/recovery/DELETED_AND_FRAGMENT_RECOVERY.md) |
| Lost directories, fragmented-tail hypotheses, damaged Word/RTF text and readable recovery editions | [Deeper recovery](docs/recovery/DEEP_RECOVERY.md) |
| Preferred generations and recoverable copy cleanup | [Recovery selection](docs/recovery/RECOVERY_SELECTION.md) |
| Automatic extraction from improved composite/FAT images | [Offline recovery handoff](docs/recovery/OFFLINE_RECOVERY_HANDOFF.md) |
| Optional hex/ASCII sectors, CHS and saved-attempt origins | [Sector inspector](docs/recovery/SECTOR_INSPECTION.md) |
| Comparing to the old archive | [Baseline comparison](docs/testing/BASELINE_COMPARISON.md) |
| Opt-in two-drive live pilot and USB shortcut | [Dual scan](docs/guides/DUAL_SCAN.md) |

This is a working **single-Greaseweazle prototype**, with live sample batches and automated tests, plus an **opt-in two-drive pilot validated on customer 007–010**, including a USB-to-GW recovery transfer. Dual mode includes persistent feeding pause/resume, explicit station actions, a rough fresh-feed ETA and durable timing exports through `fv production benchmark`. Full 136-disk acceptance, deeper damaged-filesystem recovery and broader simultaneous USB/Greaseweazle production remain targets. See [current progress](docs/development/PROGRESS.md); implementation percentages are not recovery rates.
