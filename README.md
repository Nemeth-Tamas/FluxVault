# FluxVault

**Read floppies. Keep the evidence. Recover files. Prepare a checked customer archive.**

FluxVault is a Windows-first command-line floppy archiving and recovery tool. The everyday workflow is small: insert a protected, numbered floppy, confirm it, wait for the swap banner, and repeat. Saved-file recovery, conversion, reports and lossless capture packing run automatically in new scans.

There is no GUI. Both `fv` and `fluxvault` run the same program.

## Start here

| Your experience | Open this |
| --- | --- |
| New to FluxVault or PowerShell | [Beginner tutorial](TUTORIAL.md#beginner-first-project-and-scan) |
| Set up; just need commands | [Cheat sheet](CHEATSHEET.md) |
| Want optional settings and individual tools | [Advanced tutorial](TUTORIAL.md#advanced-optional-controls) and [CLI reference](CLI.md) |
| Running a measured batch | [136-disk runbook](PILOT_136.md) or [small-test launcher](TOMORROW_TEST.md) |
| Understand a difficult saved disk | [Saved-flux diagnostics](FLUX_DIAGNOSTICS.md): `fv diagnose 59` |
| Stop active work and continue later | [Stop/resume](STOP_RESUME.md): `STOP`, `fv stop`, `fv start` |
| Check benchmark readiness | [Single-GW gates](SINGLE_GW_READINESS.md) |
| Want development status | [Progress](PROGRESS.md) and [TODO](TODO.md) |

## Beginner: the usual workflow

These examples use **PowerShell**, with `fv` already installed. Choose a **new folder name** for a new project; never initialize/reset an existing archive to resume it.

```powershell
fv init 'C:\Users\User\Desktop\FluxVault-Test\My-New-Batch'
Set-Location 'C:\Users\User\Desktop\FluxVault-Test\My-New-Batch'
fv scan --last-disk 20
```

At each prompt, insert the displayed disk with its write-protect hole open, check its label, and type its number, such as `001`. No `READ` prefix is necessary. `QUIT` stops feeding and lets saved-file work finish.

Need to cancel an active read? Type `STOP`, use Windows Ctrl+C, or run `fv stop --project 'C:\full\project\path'` in another console. Wait for **STOPPED** and drive activity to stop before moving disks. Resume the same command/project; `fv start` is an alias for `fv scan`. [Details](STOP_RESUME.md). Exit `130` means operator cancellation.

For less typing, use `fv scan --last-disk 20 --no-verify` and press **Enter only after swapping and checking the label/protection**. This skips label typing, not evidence verification.

Resume from the **same project** with the same scan command. `--last-disk 20` means stop at disk 020, not read twenty more disks. Completed recovery is reused, not automatically reread.

After scanning:

```powershell
fv status
fv recovery queue
fv conversion issues
```

The [tutorial](TUTORIAL.md) covers setup, damaged disks, interruptions and delivery step by step.

## What you need

- Windows and a built FluxVault executable. [Installation](TUTORIAL.md#1-install-or-refresh-the-short-command) requires no coding when the executable is present.
- The official Greaseweazle host tool and a working board/drive for physical scans.
- 7-Zip and LibreOffice for normal automatic saved-file processing.
- Workstation storage for the project, never a floppy as the output destination.

The validated local station is the **Mitsumi drive, straight ribbon, Greaseweazle selector B**. New scans default to B. Other wiring may need another selector; see [hardware setup](GREASEWEAZLE_PREFLIGHT.md). Greaseweazle B is **not** Windows `B:`.

New scans use supported IBM 720 KB/1.44 MB automatic identification, background processing, packed captures, four requested conversion workers and built-in recovery limits. **No policy file or format list is required.** Expert single-disk commands have different defaults; see [CLI reference](CLI.md).

Background processing and capture packing automatically share CPU/RAM/storage admission so new reads take priority. No extra settings are needed. If processing waits, `fv processing status` shows its recorded stage and resource budget; [background processing](BACKGROUND_PROCESSING.md#automatic-resource-admission) explains pressure and safe resume.

## Read the result

- **DONE / REMOVE / INSERT**: saved; swap when instructed.
- **PARTIAL SAVED / REMOVE / INSERT**: saved with unresolved sectors; swapping is still allowed.
- **RAW-ONLY FORMAT EXCEPTION SAVED**: raw evidence saved, but no compatible sector image claimed; follow its swap instruction.
- **FAILED**: do not advance the physical label. Keep the project and read the error.

FluxVault never writes to source floppies. Missing bytes are not fabricated. Fragments, carved candidates and deleted-file evidence are labeled separately. A successful command or ZIP does not prove every original file was recovered. Exit `3` means attention/partial, not necessarily a crash; exit `2` means input/operation error.

## Advanced: deeper reading

| Topic | Guide |
| --- | --- |
| Optional recovery budgets | [Policies without homework](POLICIES.md) |
| Background work and conversion workers | [Background processing](BACKGROUND_PROCESSING.md) |
| Lossless capture packing and safe restart | [Capture storage](CAPTURE_STORAGE.md) |
| Console closes/crashes: host cleanup and restart | [Process supervision](PROCESS_SUPERVISION.md) |
| Next measured 053–075 batch, single or dual drive | [Next scan](NEXT_SCAN.md) |
| Native recovery and missing boot metadata | [Damaged-filesystem recovery](DAMAGED_FILESYSTEM_RECOVERY.md) |
| Automatic orphan/signature/embedded salvage | [Carving recovery](CARVING_RECOVERY.md) |
| Fragments and opt-in deleted recovery | [Deleted and fragment recovery](DELETED_AND_FRAGMENT_RECOVERY.md) |
| Lost directories, fragmented-tail hypotheses, damaged Word streams and readable recovery editions | [Deeper recovery](DEEP_RECOVERY.md) |
| Preferred generations and recoverable copy cleanup | [Recovery selection](RECOVERY_SELECTION.md) |
| Automatic extraction from improved composite/FAT images | [Offline recovery handoff](OFFLINE_RECOVERY_HANDOFF.md) |
| Comparing to the old archive | [Baseline comparison](BASELINE_COMPARISON.md) |
| Opt-in two-drive live pilot and USB shortcut | [Dual scan](DUAL_SCAN.md) |

This is a working **single-Greaseweazle prototype**, with live sample batches and automated tests, plus an **opt-in two-drive pilot validated on customer 007–010**, including a USB-to-GW recovery transfer. Dual mode includes persistent feeding pause/resume, explicit station actions, a rough fresh-feed ETA and durable timing exports through `fv production benchmark`. Full 136-disk acceptance, deeper damaged-filesystem recovery and broader simultaneous USB/Greaseweazle production remain targets. See [current progress](PROGRESS.md); implementation percentages are not recovery rates.
