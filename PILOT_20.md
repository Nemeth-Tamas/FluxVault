# Tonight's 20-disk pilot

**One station, one command, numbered swaps.** Use the working Mitsumi on Greaseweazle selector **B**. Keep the old archive unchanged; this run creates fresh evidence and measurements in its own folder.

## Start once

Copy these into PowerShell, one line at a time:

```powershell
$fluxVaultRepo = 'C:\Users\User\Desktop\randomprojectsillneverfinish\FluxVault'
$fluxVaultPilotExe = "$fluxVaultRepo\target\release\fluxvault.exe"
$fluxVaultPilotPolicy = "$fluxVaultRepo\policies\pilot-short.json"
$fluxVaultPilotFormats = "$fluxVaultRepo\policies\customer-first-20-profiles.json"
$fluxVaultPilotProject = 'C:\Users\User\Desktop\FluxVault-Test\Customer-020-Pilot-20261006'
& $fluxVaultPilotExe init $fluxVaultPilotProject
Set-Location $fluxVaultPilotProject
& $fluxVaultPilotExe greaseweazle scan --gw-drive B --source-write-protected --last-disk 20 --policy $fluxVaultPilotPolicy --profile-map $fluxVaultPilotFormats
```

Run `init` only for a **new** folder. To resume an existing pilot, skip `init` and repeat the same scan command. Recreate the four variables if you opened a new terminal. Use the freshly built executable, not a potentially older installed copy.

The supplied format list comes from the original archive's image sizes: **009 is 720 KB; 001–008 and 010–020 are 1.44 MB**. FluxVault automatically switches at 009 and back at 010. No setting changes are needed during feeding. This list is specific to these customer numbers, not a universal format detector; do not use it for unrelated disks. Unlisted numbers use the default 1.44 MB profile.

## The only loop

1. Match the physical label to the displayed number.
2. Insert that disk with its write-protect hole **open**.
3. Type the displayed confirmation, e.g. `READ 001`.
4. Wait for **GW SWAP**, remove it, and repeat.

The command stops automatically after **020**. Do not put 021 in for this pilot. Partial results are preserved and advance normally; an operation error retains the same number and evidence. The program does not write to source floppies.

**Need to leave?** Type `QUIT` at the next swap prompt. It finishes processing already saved disks. Restart with the same command later: `--last-disk 20` always means finish at numbered disk 020, not read another twenty. Keep the same policy/format list while a disk is pending; do not change selection in a second terminal. Saved completed jobs are verified and reused rather than unnecessarily reread.

## Today's recovery budget

The supplied short policy allows at most three passes: a two-revolution whole-disk read, then increasingly thorough targeted retries only where needed, with clean control cylinders. It stops when clean, after two non-improving passes, or at its **180-second acquisition-policy ceiling**. Decode, evidence verification, publication, and downstream work have their own overhead/timeouts, so this is not an exact three-minute total wall-clock guarantee.

This is a **throughput/data-collection pilot**, not the deepest possible physical treatment. A shorter ceiling can leave sectors that a longer future reread might recover. Raw captures and unresolved-sector maps remain intact. The normal recovery default is unchanged at 600 seconds. Do not switch a pending job's policy mid-resume.

Before requesting a disk, a normal scan checks 7-Zip and LibreOffice. Missing/broken processing tools stop the scan before physical acquisition. The explicit `--acquisition-only` option is an expert fallback; it skips those checks and file processing. It is not needed for this workflow.

## After the last swap

Remove the floppy. FluxVault runs saved-image recovery/extraction, document conversion, evidence audit, and workbook export automatically. It processes after feeding, so document conversion does not delay each swap.

```powershell
& $fluxVaultPilotExe benchmark report
& $fluxVaultPilotExe status
& $fluxVaultPilotExe recovery queue
```

Look in `Reports\Benchmark` for timestamped JSON/CSV, and `Reports` for the Hungarian workbook and recovery exceptions. Timelines in `Logs\Benchmark` include the chosen per-disk profile; raw captures, image/provenance hashes, tool audits, and earlier snapshots stay local. Nothing is uploaded automatically. Keep the whole project for later analysis.

Exit **0** means the command completed cleanly; **3** means partial/attention results, not necessarily a crash; **2** means an operation/input error. These are not automatic customer-delivery certification. If saved-file processing needs a retry, run `& $fluxVaultPilotExe process`; it does not read a floppy.

**Today's acceptance:** live 007 and 009 checks → one small damaged-disk check → feed as many of 001–020 as time permits → retain benchmark and compare recovered-file hashes with the old archive. Twenty is a cap, not an obligation to finish tonight. The eventual 136-disk plan remains in [PILOT_136.md](PILOT_136.md).
