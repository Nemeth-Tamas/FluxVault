# FluxVault 1.0 — start here

Windows x64 · English operator console · Hungarian customer delivery by default

## Install without coding

Extract the **whole** portable release ZIP into a folder on your computer. Open PowerShell in that folder. You can use `./fv.exe --help` immediately, or install the short command:

```powershell
powershell -NoProfile -File .\scripts\install-cli.ps1 -AddToPath
```

Reopen your terminal, then run `fv --version`. It should say `FluxVault 1.0.0`. The installer copies the program; run it again after installing an updated release. No Rust installation or administrator session is required for installation. Windows raw USB acquisition may separately require an elevated terminal; Greaseweazle uses its official host tool.

Have the official Greaseweazle host tools, 7-Zip and LibreOffice installed. `fv tools check` reports what is available. Tool paths can be configured with `fv tools set TOOL PATH`; use `fv --help` or the beginner tutorial in `docs/guides/TUTORIAL.md` for the exact setup commands. FluxVault does not install third-party tools or change their settings automatically.

## Start a new batch

Choose a new workstation folder, never a floppy as a destination:

```powershell
fv init 'C:\Floppy-Projects\Customer-136'
Set-Location 'C:\Floppy-Projects\Customer-136'
fv production start --last-disk 136 --sound on
```

Single Greaseweazle/Mitsumi on selector B is the tested default. Insert the displayed floppy with its write-protect hole open, confirm the physical number, and follow **REMOVE / INSERT**. You type `001`, not `READ 001`. Missing sectors trigger bounded automatic recovery. Saved-file extraction/conversion runs in the background. No policy file is needed.

For Enter-only swaps add `--no-verify`. You must still physically check each label; this option does not identify the inserted floppy for you. Optional USB + GW production uses `--double`; see the dual guide for its independent station/transfer commands. Do not add a second scan against the same project.

## Stop and resume

During feeding, `QUIT` stops new reads and drains saved work. To cancel active work, use Ctrl+C or `fv stop --project 'C:\Floppy-Projects\Customer-136'` in another terminal. Wait for **STOPPED** and an idle drive before moving a disk.

```powershell
Set-Location 'C:\Floppy-Projects\Customer-136'
fv production resume --sound on
```

The saved endpoint/settings are restored. Re-add `--no-verify` if wanted. Do not initialize the project again. Exit 130 means cancellation; exit 3 means attention/partial results, not necessarily a crash; exit 2 means an error requiring investigation.

## Finish and hand over

Production finishes automatically at its verified endpoint. To finish an old `scan` batch or refresh a project after upgrading, with no floppy inserted:

```powershell
fv finish
fv package verify
```

`finish` rechecks saved images, recovery, conversions, reports and a new ZIP. It allows partial-results **archival**, retaining all warnings. It never means “every original file recovered.” Fatal integrity/path/tool errors still refuse. It creates a sibling `<project>-Delivery` destination and puts the verified ZIP, checksum, `FINAL_*` reports and `START_HERE-<edition>.html` beside `project.json`. Earlier editions remain preserved.

The customer extracts the ZIP and opens **START_HERE.html**: disk-by-disk PDF/editable links, attention notices and the full report. Only report-valid, hash-matching generated files get direct document links. Original files/evidence remain in their clearly separated folders. The page is fully offline, with no scripts or remote dependencies.

Interrupt finishing? Use `fv finish resume`. After completion, that command rechecks/publishes the saved handoff without repeating conversion or creating another ZIP. `fv finish status` only inspects historical state. To refresh newer working files, use `fv finish` again. `fv package verify` independently checks the published snapshot without modifying it.

Customer output defaults to Hungarian. Use `fv finish --language en` for an English edition, or select `--language en` when starting production. Operator prompts remain English.

## Supported v1 scope

The shipped path is **supported IBM 720 KB / 1.44 MB → read-only acquisition → bounded recovery → FAT12 extraction and separate damaged-file candidates → supported Office/PDF conversion → reports → verified archival ZIP**. One GW is default; one USB + one GW is optional. Original captures, gaps/conflicts and separate recovery candidates retain provenance. No manual DMDE step is required for this supported path.

Rare formats, broader damaged-document reconstruction, proof of semantic completeness, universal every-boundary power-loss acceptance and future extra-drive modes remain post-v1 work—not hidden promises or completed checklist items. Unknown/password-protected/unrecoverable results remain attention. Physical USB write blocking depends on independently qualified hardware; the program opens source devices read-only but cannot certify an adapter's electronics.

## More help

`fv --help` lists the current commands. `FLUXVAULT-MANUAL.txt` is the plain-text Windows manual. The `docs/guides` folder includes beginner/advanced instructions, stop/resume, dual stations and expert recovery commands. The source repository is https://github.com/Nemeth-Tamas/FluxVault.
