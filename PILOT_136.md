# The first 136-disk pilot

**Current checkpoint:** the first 20-disk pilot is complete. New projects identify supported IBM formats, pack captures and process files in the background. Inconclusive completed format trials become preserved raw-only exceptions so feeding can continue. Tool/integrity failures still stop. Run the short live gates on this build before the full session; [PILOT_20.md](PILOT_20.md) contains historical measurements.

**Goal:** read the numbered customer collection on the working Mitsumi/Greaseweazle station, preserve evidence, run the saved-file processing chain, and collect a useful performance/recovery baseline. This is a controlled single-station pilot, not a promise that every damaged disk will yield every file or that the six-hour dual-drive target has been met.

## 1. Start with a small hardware smoke test

For the shortest next-session path, use [TOMORROW_TEST.md](TOMORROW_TEST.md): its launcher creates an isolated project, runs 009 or 058, and gathers a pasteable summary and baseline comparison. The manual steps below remain available.

Use a **new project folder**, separate from the old script archive and earlier test captures. Never reset or recreate `FluxVault-Test`. A fresh project ensures this is a fresh physical-read benchmark, not reuse of earlier completed recovery jobs.

Use the freshly built executable directly, so an older installed copy cannot accidentally be used:

```powershell
$fluxVaultPilotExe = 'C:\Users\User\Desktop\randomprojectsillneverfinish\FluxVault\target\release\fluxvault.exe'
& $fluxVaultPilotExe init 'C:\Users\User\Desktop\FluxVault-Test\Pilot-Smoke-007'
Set-Location 'C:\Users\User\Desktop\FluxVault-Test\Pilot-Smoke-007'
& $fluxVaultPilotExe tools check
& $fluxVaultPilotExe greaseweazle info
& $fluxVaultPilotExe disk select 7
& $fluxVaultPilotExe greaseweazle scan --gw-drive B --source-write-protected --last-disk 7
```

The example assumes the folder does not exist; choose a fresh name otherwise. Customer 007's archive was clean, not a guarantee of a clean new read. Insert with the protection hole open and confirm **`007`** after checking the label. On **GW SWAP**, remove it. Also test 009 in a fresh smoke project (`disk select 9`, `scan --last-disk 9`) to exercise automatic DD discovery and packed storage. Neither test should use the old pilot folder for a new physical read.

Before committing to the full collection, also exercise a damaged disk in a separate smoke-test project, inspect its preserved partial result, and verify that file processing and the benchmark exports finish. Keep smoke tests separate from the full pilot's numbered sequence. If you want me to operate the live test, tell me which numbered floppy is inserted first.

## 2. Run the collection in a fresh pilot project

After the short hardware checks pass:

```powershell
& $fluxVaultPilotExe init 'C:\Users\User\Desktop\FluxVault-Test\Customer-136-Pilot'
Set-Location 'C:\Users\User\Desktop\FluxVault-Test\Customer-136-Pilot'
& $fluxVaultPilotExe scan --last-disk 136
```

Check the disk label against every prompt. Insert the protected disk, type the displayed **number only** (e.g. `004`), and wait for **GW SWAP** before removing it. Completed partial results advance normally. An acquisition error keeps the same number/evidence. New scans process saved files between swaps; **`QUIT`** ends feeding and drains/reconciles work. The source is never written. Plain `scan` reuses saved settings on restart.

For Enter-only swaps, add `--no-verify` each invocation: it skips label typing, not protection/evidence verification. Never press Enter until the swap and label/protection checks are complete. Four conversion workers remains conservative; `--conversion-workers N` saves a request capped in background mode to leave two logical CPUs for acquisition. After 136, **BATCH FINISHED / REMOVE 136** requests no further insertion. See [background work](BACKGROUND_PROCESSING.md).

In a second terminal, `fv processing status` shows saved-file progress without disturbing the swap prompt. After an interruption, `fv processing resume` drains image work offline and `fv storage resume` drains packing. Another mutating report/conversion command is refused while a worker owns the project. Older scan journals retain serial tail mode; `--processing-mode background` opts them in, while `--processing-mode tail` opts out.

`--last-disk 136` means **stop after numbered disk 136**, not “read another 136 disks.” Repeat that exact command after a restart; no remaining-disk arithmetic is needed. An interrupted physical job asks you to reconfirm the same numbered disk; an interrupted completed-numbering commit is reconciled without another physical read. Keep the same profile, drive, policy and end target for a pending job. Do not change disk selection from another terminal.

New projects default to **automatic IBM 720 KB/1.44 MB discovery**, with bounded passes. Nearly complete coherent HD needs one offline decode; otherwise both profiles are tried without another physical read. If both complete but cannot select a convincing format, raw evidence and trial reports are verified and saved as `raw_format_exception`; an amber cue permits removal/next insertion, with no fake sector image or known bad-sector count. This remains attention in status, recovery queue, processing and benchmark summaries, and blocks an automatic all-clear package. Decoder/tool failures still stop. Fixed profiles/maps remain expert controls; do not change a pending job's mode.

Archive inspection found 134 HD-sized images, 009 DD-sized, and a 417,792-byte image for 133; the latter does not establish its physical geometry. Supported automatic discovery has passed saved 007/009 tests, not universal format acceptance. Use a short 009 live check to validate the new path before scaling up. Nonstandard/severely damaged 133 handling remains open.

New scans default to verified packed retention. One background worker packs completed captures while swaps continue; original SCP size/hash and all bytes survive in `.scp.zip` plus binding metadata. Raw working copies retire only after verification. `--capture-storage raw` opts out; old journals keep raw storage. `storage resume` finishes pending packing offline. Original pilot/archive evidence stays separate. On a copy of all 26 captures, this reclaimed 76.42% of raw working space; it is not a new acquisition-throughput measurement.

## 3. Read the benchmark

Every scan automatically records telemetry and exports a snapshot at a normal session end. To regenerate a snapshot later, including after an interruption:

```powershell
& $fluxVaultPilotExe benchmark report
& $fluxVaultPilotExe status
& $fluxVaultPilotExe recovery queue
```

The benchmark command uses saved telemetry only. It does not query the board, read a floppy, or start extraction/conversion. Exports have unique names, so earlier snapshots remain intact.

| Local output | Purpose |
| --- | --- |
| `Reports\Benchmark\Benchmark-*.json` | Session configuration/build fingerprint, disk outcomes, failures, timings and downstream summaries |
| `Reports\Benchmark\Benchmark-*.csv` | One row per committed disk: status, bad-sector counts, read/wait times and image hash |
| `Logs\Benchmark\.fluxvault-benchmark-*.jsonl` | Append-only, synced event timeline for each invocation |
| `Logs\external-tools.jsonl` | Host versions, commands, process durations and detailed external-tool output |
| `Logs\ProcessingEvents.jsonl` / `Reports\ProcessingStatus.json` | Quiet background stages, elapsed time, worker counts and final outcome |
| `.fluxvault-processing/jobs/*.json` | Durable verified-image jobs and attention/failed states |
| `.fluxvault-gw-scan.json` | Durable scan custody/completion history and pending disk |

Recorded measurements include confirmation wait time, acquisition/decode/publication time, subsequent evidence-verification time, reported physical read operations, stop reason, missing/conflicting LBAs, per-sector confidence counts, image/provenance hashes, failures and downstream processing results. Mean/median/95th-percentile recovery times describe the observed timed jobs. Resumed numbering does not inflate unique disk counts or physical-read totals.

The 136-disk feed-time projection extrapolates the observed successful physical-job sample, confirmation waits and evidence verification. It excludes failed attempts, final downstream time, some journal/export overhead and time between sessions; it is **not** a full-job completion estimate. Finished-session wall time and separate failed/downstream durations are also retained. An incomplete session may be interrupted or still active. An uncommitted trailing event is flagged and ignored; malformed committed records are refused rather than silently fabricated. Read operations that failed before returning a recovery result are not included in `reported_physical_reads`; consult the tool audit for those.

## 4. Data for the next development pass

Keep the entire pilot project locally. The useful first review set is the latest benchmark JSON/CSV, raw benchmark event files, external-tool audit, scan journal, evidence audit and recovery exceptions. These files do not include customer file contents, but paths/tool output may contain private information. Review them before sharing; nothing is uploaded automatically. Benchmark files are internal working material and are excluded from customer packages.

We can use the pilot to identify slow passes, repeatedly unreadable LBAs, low-yield recovery patterns, avoidable operator confirmations and downstream bottlenecks. The old script archive stays unchanged as the comparison baseline. Matching recovered-file counts/hashes against that archive is a separate acceptance step, not something inferred from a clean sector count.

**Acceptance order:** small clean/damaged live checks → first small numbered cohort → 136-disk run → compare baseline recovery and elapsed time → tune policy and background scheduling from measured data.
