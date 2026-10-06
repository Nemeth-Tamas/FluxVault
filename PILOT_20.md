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

Run `init` only for a **new** folder. After the first numbered confirmation saves the settings, resume with just `& $fluxVaultPilotExe scan`: it remembers the drive, formats, policy and endpoint. If installed/aliased as `fv`, simply use `fv scan`. Use the freshly built executable, not a potentially older installed copy. [Policies](POLICIES.md) are optional expert controls; the existing customer pilot already has its settings saved.

The supplied format list comes from the original archive's image sizes: **009 is 720 KB; 001–008 and 010–020 are 1.44 MB**. FluxVault automatically switches at 009 and back at 010. No setting changes are needed during feeding. This list is specific to these customer numbers, not a universal format detector; do not use it for unrelated disks. Unlisted numbers use the default 1.44 MB profile.

## The only loop

1. Match the physical label to the displayed number.
2. Insert that disk with its write-protect hole **open**.
3. Type the displayed number, e.g. `001` (no `READ` needed).
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

## First completed cohort — 2026-10-06

The operator completed customer **001–020** using number-only confirmations and the saved short policy. After the initial disk-004 host failure, the resumed scan completed without losing the earlier acquisitions. The operator approves the simplicity/custody workflow; the next usability priority is an unmistakable colored swap banner, because the current prompt is easy to miss.

| Recorded outcome | Result |
| --- | --- |
| Committed disks | 20: 17 clean, 3 partial |
| Partial disks | 005, 012, 017: one missing sector each; zero conflicting sectors |
| Native partial-image extraction | 005: 3 complete files; 012: 11; 017: 15, with one skipped entry |
| Extraction / conversion | 20 disks extracted; 172 conversion jobs succeeded, zero failed |
| Saved raw captures | 26: twenty initial captures plus six targeted rereads; the failed no-file attempt is separate |
| Mean / median recovery time | 108.16 / 104.97 seconds per committed disk |
| Recorded successful recovery / operator waits | 36.05 / 13.15 minutes across the two invocations |
| Downstream tail | 7.89 minutes, using four conversion workers |
| Raw SCP / complete working project | 985.13 MiB / approximately 1.08 GiB |

These timings are measured stages, not the elapsed time between first launch and final completion; troubleshooting/restart downtime is not fully represented. The benchmark estimates **5.52 hours of feeding for 136 disks** with this cohort's policy/behavior. It does not validate a 136-disk full-chain afternoon, deeper recovery or dual-drive throughput.

The local evidence snapshot is `Reports/Benchmark/Benchmark-1791283405279683100-25748.json` plus its per-disk CSV, and `Reports/FluxVault_Jelentes_20261006_124325_267.xlsx`. There are two scan sessions (one interrupted, one finished) and one genuine initial acquisition error. The current benchmark also counts downstream exit **3** as an error: here it means three partial disks needing attention, not failed extraction/conversion. Splitting those categories is tracked in the TODO.

Partial disks remain partial even when intact files can be extracted. In particular, disk 017 had one skipped entry. Cohort-wide comparison with the old script/DMDE output, including carved/deleted files, remains necessary before claiming equal recovery yield or customer-delivery certification.

Conversion is already threaded: a shared queue feeds the next job to whichever worker finishes. `process --conversion-workers 12` is supported today (valid range 1–16), but the automatic scan tail currently uses four; exposing a saved scan worker setting and smarter mixed-size scheduling is planned. More workers require a measured CPU/RAM budget, not an assumed linear speedup.

Most space is physical multi-revolution flux evidence, not extracted documents: 26 SCP files account for 985.13 MiB, while `Converted` is only 34.24 MiB. The old sector-image ZIP is not an equivalent storage comparison. Planned lossless background compression must verify byte-identical decompression, preserve provenance/resume, and avoid competing with acquisition; no captures are removed by this documentation update.

## Offline operator-preparation checkpoint — 2026-10-06

Large ASCII/color swap banners and opt-in `scan --no-verify` are now implemented. Enter-only mode warns at every prompt, retains read-only/evidence checks and never persists into later invocations. `scan --conversion-workers 12` now saves/forwards the tail worker count. The queue interleaves estimated long and short jobs; conversion planning/state have project ownership and immutable snapshot history.

An offline rerun exposed a pre-existing Windows path-identity bug: DOS and extended/canonical paths lost their conversion reuse bindings. The fix compares actual path identity while still requiring matching source/output hashes. The existing generated files and report state were preserved in `C:\Users\User\Desktop\FluxVault-Test\Conversion-before-path-fix-20261006` before rebuilding fresh outputs; captures, images and extracted sources were not replaced.

The fresh 12-worker run returned 164 OK/eight partial DOCX timeouts; the restricted desktop runner reported denied process-tree termination. A saved-issue retry at four workers outside that restriction restored **172 OK, zero conversion exceptions**, reusing 336 outputs. No owned LibreOffice processes remained. A subsequent extended-path `process --conversion-workers 12` reused the entire converted set with **zero new conversion invocations**, leaving 17 evidence-verified disks and the same three partial disks. Four remains the conservative default; higher worker counts need a controlled throughput/resource benchmark.

`storage benchmark 7` measured 54,050,828 raw bytes to 12,343,912 ZIP/Deflate bytes at level 6 (**77.16% smaller**), approximately 0.86 seconds compression / 0.07 seconds roundtrip verification. Source and decompressed hashes matched exactly. This measures one full capture in memory; no working capture has been packed/deleted. Customer ZIP entries now use lossless compression rather than uncompressed storage.

The full offline archival-package test then included all **26 raw captures**, derived evidence, originals, converted files and selected reports: **937 manifest-verified files**, **266.19 MiB ZIP**. The raw SCP entries alone compressed from **985.13 to 232.24 MiB (76.42% smaller)**. The local artifact is under `FluxVault-Test/Offline-Prep-Packages-20261006`; it contains no conversion-state history or benchmark internals. It remains an archival test package, not certification of the three partial disks. Working captures stay uncompressed until transparent background packing/decode/resume is implemented.
