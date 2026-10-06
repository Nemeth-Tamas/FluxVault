# FluxVault command cheat sheet

**For the person swapping floppies, not writing software.** These examples are for **PowerShell on Windows**. Copy one line at a time. FluxVault reads source floppies; its images, logs, and reports are written to the project folder.

> [!IMPORTANT]
> `A:` is the **Windows USB floppy drive**. Greaseweazle's `--gw-drive A` is a **different drive selector**. Do not use a customer disk to test write protection. The USB acquisition commands require an independently verified write blocker and a positive Windows protection report; the flag does not bypass either check.

## 1. Get to a working prompt

The current built executable is in the repository. To use it from any folder as `fluxvault`, run this once from PowerShell:

```powershell
Set-Location 'C:\Users\User\Desktop\randomprojectsillneverfinish\FluxVault'
powershell -NoProfile -File .\scripts\install-cli.ps1 -AddToPath
```

**Close and reopen PowerShell** after installation, then check:

```powershell
fluxvault --help
```

If the installer says the release executable is missing, run `cargo build --release` in the repository first. If you do not want to install anything, run `.\target\release\fluxvault.exe` from the repository instead of `fluxvault` in the examples below.

## 2. Open the existing test project

This project **already exists**. Do not run `init` on it.

```powershell
Set-Location 'C:\Users\User\Desktop\FluxVault-Test'
fluxvault status
fluxvault disk list
fluxvault recovery queue
```

FluxVault finds the project from the current folder or one of its subfolders. From elsewhere, add `--project 'C:\Users\User\Desktop\FluxVault-Test'` to a command.

For a **brand-new** project in a different location:

```powershell
fluxvault init 'C:\path\to\New-Archive'
Set-Location 'C:\path\to\New-Archive'
fluxvault status
```

## 3. The everyday floppy loop (USB drive)

First confirm which drive Windows sees. `drive list` does not read a disk; `drive probe` reads only the first 512 bytes, without writing.

```powershell
fluxvault drive list
fluxvault drive probe --drive A:
```

Only after independent hardware-protection verification, insert a physically write-protected floppy and scan it. For a **known numbered disk**, this makes a new, numbered image attempt; it does not overwrite an earlier attempt:

```powershell
fluxvault acquire --drive A: --disk 7 --retries 2 --write-blocker-verified
fluxvault disk show 7 --details
```

For a **sequence of disks**, select the starting number, then let FluxVault prompt after each swap:

```powershell
fluxvault disk select 8
fluxvault scan --drive A: --count 10 --retries 2 --write-blocker-verified
```

At each prompt, insert the correctly numbered floppy and type `READ`. Type `QUIT` to end the session. A partial image still advances the sequence and enters the recovery queue; a failed acquisition does not advance it. `--count 10` is a safety cap, not a requirement to have ten disks ready. Omit it to keep going until `QUIT`.

> [!NOTE]
> `--retries 2` means two bounded retry passes after the initial read. FluxVault will refuse acquisition if Windows does not report the floppy as protected. Do not try to work around that with customer media.

## 4. What to type after scanning

| You want to... | Type this |
| --- | --- |
| See overall progress | `fluxvault status` |
| See all imaged disks | `fluxvault disk list` |
| Inspect one disk and its bad sectors | `fluxvault disk show 7 --details` |
| See which disks need attention | `fluxvault recovery queue` |
| See suggested recovery actions | `fluxvault recovery plan` |
| Compare repeat USB reads of disk 7 | `fluxvault recovery compare 7` |
| Verify saved image/extraction evidence | `fluxvault audit` |

The following commands operate on **saved images and files**, not the physical floppy:

```powershell
fluxvault extract all
fluxvault files manifest
fluxvault conversion plan
fluxvault conversion run
fluxvault conversion issues
fluxvault report export
```

`extract all` needs 7-Zip. `conversion run` needs LibreOffice. Check what FluxVault can find with `fluxvault tools check`; `fluxvault tools show` shows configured paths. If a tool is installed somewhere unusual, use `fluxvault tools set sevenzip 'C:\path\to\7z.exe'` or `fluxvault tools set libreoffice 'C:\path\to\soffice.exe'`.

For the saved-image processing chain in one command, use:

```powershell
fluxvault process
```

This runs recovery/extraction, conversion, audit, and report work. It does **not** guarantee every damaged floppy has yielded every original file. Review the recovery queue, conversion issues, and audit before delivery.

## 5. When a disk has bad sectors

Start with the evidence; do not guess missing bytes:

```powershell
fluxvault disk show 7 --details
fluxvault recovery plan 7
fluxvault recovery compare 7
```

If there are multiple saved USB attempts, the guarded offline recovery commands can preserve a backup and try evidence-based reconstruction:

```powershell
fluxvault recovery backup 7
fluxvault recovery composite 7
fluxvault recovery fat 7
fluxvault extract disk 7
```

`composite` uses corroborated saved-sector evidence; `fat` reconstructs only provable mirrored FAT sectors. Neither is permission to invent or silently certify damaged customer data. Some disks will still need attention.

To recover intact files from a damaged **saved** FAT12 image, without inserting the disk:

```powershell
fluxvault recovery extract 7
```

`process` and the extraction commands now try this automatically too. Standalone `recovery extract` needs no 7-Zip or LibreOffice. It saves complete readable files separately, leaves old extraction/manual files alone, and records what it skipped. A successful result still returns **3 (partial/attention)**—that is expected. It does not invent unreadable bytes or reconstruct deleted files/long names yet. See `Recovery\007\attempt_NNN_fat12.json` and `Reports\RecoveryExceptions.txt` for the exceptions (the latter is generated by batch `extract all`/`process`).

## 6. Greaseweazle: insert one disk, run one command

The tested **Mitsumi drive with the straight ribbon uses selector B**. The original NEC is faulty; do not use it as the production drive. See [setup and live checks](GREASEWEAZLE_PREFLIGHT.md). Check the host/board after connecting:

```powershell
fluxvault tools check
fluxvault greaseweazle info
fluxvault greaseweazle preview
```

Insert the correctly numbered, protected **1.44 MB** floppy. No USB scan is required first:

```powershell
fluxvault greaseweazle recover 7 --gw-drive B --source-write-protected
```

FluxVault captures and decodes, rereads only problem areas within bounded limits, then runs extraction/conversion/audit/report processing. After it finishes, remove the disk and run the command with the next disk's number. It stops after two non-improving passes rather than endlessly hammering the disk.

For verified **720 KB** media, add `--profile ibm.720`. Format discovery is not automatic yet; the default is 1.44 MB. Numbering/swapping is still manual here; a guided Greaseweazle batch loop is planned. Keep only one physical Greaseweazle command running at a time.

Repeat the same command/settings to resume an interrupted job. A completed job is verified/reused, not physically reread. Partial results keep missing sectors explicit—no bytes are guessed. The downstream pipeline now salvages intact reachable FAT12 files automatically, but missing metadata/deleted files/carving remain unfinished. Clean maps can still contain lower-confidence single-capture sectors; they are not delivery certification. Add `--acquisition-only` to skip downstream processing.

Optional **offline** evidence checks:

```powershell
fluxvault greaseweazle status 7
fluxvault disk show 7 --details
fluxvault recovery queue
```

Expert separate capture/decode commands remain available:

```powershell
fluxvault greaseweazle capture 7 --profile ibm.1440 --gw-drive B --source-write-protected
fluxvault greaseweazle decode 7
fluxvault greaseweazle consensus 7
```

Only `capture` in that last block accesses the disk. `consensus` requires two distinct saved raw captures; repeated decodes of one SCP do not count. `recover` already records confidence/provenance automatically; these expert commands are not required for its normal use.

## 7. Delivery, only when checks are clear

Create the destination folder **outside** the project first. `finalize` processes the saved images and only packages when recovery, conversion, and audit allow it:

```powershell
New-Item -ItemType Directory -Path 'C:\CustomerPackages' -Force
fluxvault finalize --destination 'C:\CustomerPackages'
```

If it reports attention, inspect `recovery queue`, `conversion issues`, and `audit`. Do not treat an archival ZIP as proof that all source bytes were recovered.

## Quick interpretation

| Result | Meaning |
| --- | --- |
| Exit code `0` | Command completed. |
| Exit code `3` | Partial result or attention needed; **not necessarily a crash**. |
| Exit code `2` | Invalid input or operation error; read the message. |

Useful extras: `fluxvault --help` lists every command; `--json` gives machine-readable output. For detailed behavior and limitations, see [CLI.md](CLI.md) and [TODO.md](TODO.md).
