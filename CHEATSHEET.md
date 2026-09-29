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

## 6. When the Greaseweazle arrives

Follow [the physical preflight checklist](GREASEWEAZLE_PREFLIGHT.md) **first**, with a disposable, write-protected disk. The Greaseweazle capture path has not yet been validated on the actual board/NEC drive. `tools check` only checks the host program; `greaseweazle info` checks for a connected, ready board.

```powershell
fluxvault tools check
fluxvault greaseweazle info
fluxvault greaseweazle preview
```

After the disposable-disk preflight, an example **read-only raw-flux capture** for disk 7 is:

```powershell
fluxvault greaseweazle capture 7 --gw-drive A --source-write-protected
```

Then remove the floppy. These next commands work **offline** on saved evidence:

```powershell
fluxvault greaseweazle decode 7
fluxvault greaseweazle status 7
fluxvault greaseweazle compare 7
```

After **two distinct physical flux captures**, also run:

```powershell
fluxvault greaseweazle consensus 7
fluxvault greaseweazle plan 7
```

These reports identify candidates and conflicts. They do **not** automatically put flux-decoded sectors into a certified customer image. If FluxVault cannot infer the format from a saved USB image, capture requires `--profile ibm.1440` or `--profile ibm.720`, chosen from independently verified disk format.

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
