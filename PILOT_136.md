# The first 136-disk pilot

**Goal:** read the numbered customer collection on the working Mitsumi/Greaseweazle station, preserve evidence, run the saved-file processing chain, and collect a useful performance/recovery baseline. This is a controlled single-station pilot, not a promise that every damaged disk will yield every file or that the six-hour dual-drive target has been met.

## 1. Start with a small hardware smoke test

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

The example assumes the smoke-test folder does not already exist. Choose a new name if it does. Customer 007's archived acquisition was clean; it is a useful initial comparison disk, not a guarantee of a clean new read. Insert it only when ready for the physical test, with its write-protect hole open. Confirm **`READ 007`** only after checking the actual label. On **GW SWAP**, remove it.

Before committing to the full collection, also exercise a damaged disk in a separate smoke-test project, inspect its preserved partial result, and verify that file processing and the benchmark exports finish. Keep smoke tests separate from the full pilot's numbered sequence. If you want me to operate the live test, tell me which numbered floppy is inserted first.

## 2. Run the collection in a fresh pilot project

After the short hardware checks pass:

```powershell
& $fluxVaultPilotExe init 'C:\Users\User\Desktop\FluxVault-Test\Customer-136-Pilot'
Set-Location 'C:\Users\User\Desktop\FluxVault-Test\Customer-136-Pilot'
& $fluxVaultPilotExe greaseweazle scan --gw-drive B --source-write-protected --last-disk 136
```

Check the disk label against every prompt. Insert the protected disk, type the displayed **`READ NNN`**, and wait for **GW SWAP** before removing it. Completed partial results advance normally. An acquisition error keeps the same disk selected and preserves evidence. **`QUIT`** ends feeding early and runs the saved-file processing tail. The source floppy is never written by FluxVault.

`--last-disk 136` means **stop after numbered disk 136**, not “read another 136 disks.” Repeat that exact command after a restart; no remaining-disk arithmetic is needed. An interrupted physical job asks you to reconfirm the same numbered disk; an interrupted completed-numbering commit is reconciled without another physical read. Keep the same profile, drive, policy and end target for a pending job. Do not change disk selection from another terminal.

Default format is **1.44 MB**, with bounded Fast/Normal/Recovery/Detective passes. For a known 720 KB cohort, use `--profile ibm.720` in a separate smoke test/session. This pilot does not yet choose formats automatically. A raw capture remains preserved even when the selected decoder cannot recover useful sectors; such a result is not proof the underlying floppy contains no recoverable data.

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
| `.fluxvault-gw-scan.json` | Durable scan custody/completion history and pending disk |

Recorded measurements include confirmation wait time, acquisition/decode/publication time, subsequent evidence-verification time, reported physical read operations, stop reason, missing/conflicting LBAs, per-sector confidence counts, image/provenance hashes, failures and downstream processing results. Mean/median/95th-percentile recovery times describe the observed timed jobs. Resumed numbering does not inflate unique disk counts or physical-read totals.

The 136-disk feed-time projection extrapolates the observed successful physical-job sample, confirmation waits and evidence verification. It excludes failed attempts, final downstream time, some journal/export overhead and time between sessions; it is **not** a full-job completion estimate. Finished-session wall time and separate failed/downstream durations are also retained. An incomplete session may be interrupted or still active. An uncommitted trailing event is flagged and ignored; malformed committed records are refused rather than silently fabricated. Read operations that failed before returning a recovery result are not included in `reported_physical_reads`; consult the tool audit for those.

## 4. Data for the next development pass

Keep the entire pilot project locally. The useful first review set is the latest benchmark JSON/CSV, raw benchmark event files, external-tool audit, scan journal, evidence audit and recovery exceptions. These files do not include customer file contents, but paths/tool output may contain private information. Review them before sharing; nothing is uploaded automatically. Benchmark files are internal working material and are excluded from customer packages.

We can use the pilot to identify slow passes, repeatedly unreadable LBAs, low-yield recovery patterns, avoidable operator confirmations and downstream bottlenecks. The old script archive stays unchanged as the comparison baseline. Matching recovered-file counts/hashes against that archive is a separate acceptance step, not something inferred from a clean sector count.

**Acceptance order:** small clean/damaged live checks → first small numbered cohort → 136-disk run → compare baseline recovery and elapsed time → tune policy and background scheduling from measured data.
