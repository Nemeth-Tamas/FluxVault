# FluxVault CLI

## Everyday short workflow

The installer provides both `fluxvault.exe` and the identical short alias `fv.exe`. `fv init`, `fv status`, and `fv scan` work from the project folder. Plain `scan` uses Greaseweazle; explicit `scan --drive A:` (or USB protection/retry flags) retains the existing guarded USB workflow.

GW scans reuse saved project-journal defaults for omitted profile, profile map, selector, policy and end target; explicit expert arguments override them, subject to pending-job consistency checks. Fresh projects currently default to selector B, ibm.1440, the normal built-in policy and no endpoint. Per-disk numbered input asserts label/protection checks, so a separate `--source-write-protected` flag is unnecessary for this guided loop; individual capture/recover still require it. `004` or `4` confirms disk 004; `QUIT`/`Q` stops. Empty/wrong-number input does not read. Legacy `READ 004` remains accepted. See [POLICIES.md](POLICIES.md).

The runner treats explicit `Command Failed:`/`ERROR:`/`Fatal Error:` output as operation failure even if the host exits zero. In particular, `No Index` preserves failed metadata and gives reseating guidance rather than a misleading missing-SCP error. A resumed empty first capture (no raw artifact or completed raw record/decode) may restart its elapsed acquisition budget after explicit custody confirmation; old budget starts and failed attempt files are retained. Any full/partial raw evidence blocks that reset. Completed disks/cursor are preserved.

For a copy-and-paste operator guide, start with [CHEATSHEET.md](CHEATSHEET.md).

For setup and the recorded live checks, see [GREASEWEAZLE_PREFLIGHT.md](GREASEWEAZLE_PREFLIGHT.md). The working shop drive is the Mitsumi on selector **B**; the original NEC has a faulty head/read path.

Build with `cargo build --release`; the executable is `target\release\fluxvault.exe` on Windows. To install a copy and optionally add its directory to your user `PATH`, run `powershell -NoProfile -File .\scripts\install-cli.ps1 -AddToPath` from the repository root; omit `-AddToPath` to copy without changing PATH, or add `-WhatIf` to preview. Open a new terminal after a PATH change. Running `fluxvault` without arguments shows command help; the desktop GUI has been removed. The release executable and installer were previously checked from PowerShell/CMD and with a disposable directory. No installation or PATH change was performed in your user profile.

From a project folder (or any subfolder), for example:

```powershell
fluxvault status
fluxvault benchmark report
fluxvault disk list
fluxvault disk show 7 --details
fluxvault recovery plan
fluxvault recovery queue
fluxvault recovery compare 7
fluxvault recovery backup 7
fluxvault recovery composite 7
fluxvault recovery fat 7
fluxvault recovery extract 7
fluxvault tools check
fluxvault tools show
fluxvault greaseweazle preview
fluxvault greaseweazle info
fluxvault greaseweazle recover 7 --gw-drive B --source-write-protected
fluxvault greaseweazle scan --gw-drive B --source-write-protected --count 10
fluxvault greaseweazle capture 7 --gw-drive A --source-write-protected
fluxvault greaseweazle decode 7
fluxvault greaseweazle status 7
fluxvault greaseweazle compare 7
fluxvault greaseweazle consensus 7
fluxvault greaseweazle plan 7
fluxvault extract all
fluxvault extract disk 7
fluxvault files manifest
fluxvault conversion plan
fluxvault conversion run
fluxvault conversion issues
fluxvault conversion retry
fluxvault conversion retry C:\path\to\Extracted\001\problem.rtf
fluxvault report export
fluxvault audit
fluxvault package build --destination C:\CustomerPackages
fluxvault finalize --destination C:\CustomerPackages
```

Use `--project C:\path\to\project` to select a project explicitly. Add `--json` to a command for machine-readable stdout (including structured errors); long-running progress goes to stderr. Exit code 0 means complete, 3 means attention/partial, and 2 means invalid input or an operation error. These codes will be refined as production automation is added.

`disk show N` summarizes saved attempts. Add `--details` for their hashes, bad-sector LBAs, retry counts, and evidence paths; JSON includes those fields without an extra flag. Neither view accesses the floppy drive.

`fluxvault drive list` enumerates removable drives without reading inserted media. `fluxvault drive probe --drive A:` opens only an enumerated drive read-only, reads at most the first 512 bytes, and reports geometry and the Windows write-protection result. A positive software result is **not proof that this USB adapter enforces physical write protection**. `fluxvault acquire --drive A: --disk N --retries 2 --write-blocker-verified` requires an operator hardware-protection assertion, a positive Windows protection report, and plausible floppy geometry; the imaging backend checks protection again when it opens the drive read-only. On 2026-09-26, Windows reported `protected` for customer floppy 007 and the CLI completed a read-only 1.44 MB acquisition. That attempt had one unresolved sector; all other 2,879 sectors matched the earlier clean archived image byte-for-byte. This validates the CLI read path, not the adapter's physical write-blocking behavior.

`fluxvault scan --drive A: --write-blocker-verified` runs a guided single-drive loop from the project's current disk number. After each physical swap, type `READ` to image the inserted disk or `QUIT` to stop; other input does not start a read. `--count N` caps the number of disks in that session, and `--retries N` sets the same bounded sector retries as `acquire`. A completed partial image advances numbering and appears in the recovery queue; a failed acquisition does not advance it. Scan prompts and progress use stderr, leaving final `--json` output machine-readable. The multi-disk loop itself has only been tested with synthetic acquisitions; the one-disk `acquire` path has been tested live as described above.

`fluxvault tools check` runs 7-Zip, LibreOffice, and Greaseweazle version checks and records executed commands in the project tool audit log (or the application audit log when no project is selected). `tools show`, `tools set NAME PATH`, and `tools clear NAME` manage per-user tool paths. `greaseweazle preview` prints safe raw-capture and file-to-file decode command examples without executing anything; `greaseweazle info` runs an audited, read-only device/firmware query. `recovery queue` shows unfinished cases; `recovery compare N`, `recovery backup N`, `recovery composite N`, and `recovery fat N` operate on saved evidence. `recovery import N --source DIR --dmde-log FILE` copies external DMDE results into guarded project recovery locations without overwriting an earlier import. `extract all` and `extract disk N` need 7-Zip, not LibreOffice. `files manifest` refreshes the recovered-file inventory. `conversion plan` builds delivery paths without LibreOffice; `conversion run` executes bounded, audited Office conversion. `conversion issues` reads saved exceptions. `conversion retry [SOURCE]` reloads the project-scoped conversion state after a restart, retries all saved issues or the selected source, and rejects changed source hashes or paths. `process` runs the existing-image recovery, extraction, conversion, audit, and workbook pipeline.

`tools check` confirms the **host program** is callable, not that a Greaseweazle board is attached. `greaseweazle info` now parses the device section and reports `device_status`/`ready` in JSON; an absent or unverified board returns attention exit code 3 even if `gw info` itself exits 0. Raw capture performs the same read-only info preflight before reserving an attempt or issuing `gw read`. This matters because upstream `gw info` explicitly prints `Device: Not found` and exits 0 in that case. [Greaseweazle info source](https://github.com/keirf/greaseweazle/blob/master/src/greaseweazle/tools/info.py)

Tool-health/version probes have short process deadlines (15 seconds for Greaseweazle and 7-Zip, 30 seconds for LibreOffice) and retain an audit entry on success or timeout. Greaseweazle host version comes from the **Host Tools** section of `gw info`, not an unsupported `--version` flag; normal output may arrive on stderr. The additional host-version probe is bounded to 10 seconds; `info`, `read`, and `convert` retain separate 15/300/60-second defaults. Automatic recovery further bounds each stage by remaining policy time.

The capture/decode path was **live-tested on 2026-10-05** with V4.1, host 1.23, firmware 1.6, the Mitsumi D353M3D-5056 on selector B, and protected WinWord 1. Tool health, board info, two full raw captures, offline decode, status, consensus, targeted automatic recovery, and downstream audit/report processing passed; one sector (LBA 24) remains unreadable. This does not validate all formats/media or prove hardware write blocking. `greaseweazle capture N --source-write-protected` runs only `gw read` with `--raw` and `--no-clobber`, preserving numbered SCP, SHA-256, metadata, and audit. The flag records your physical-tab confirmation. Format is inferred from saved 1.44 MB/720 KB USB evidence when possible; otherwise pass `--profile ibm.1440` or `--profile ibm.720`. Selector A remains the generic default; **use B for the current straight cable**. `--revs N` accepts 1–10, default 3. Selectors are not Windows drive letters.

### Automatic Greaseweazle-only recovery

```powershell
fluxvault greaseweazle recover 7 --gw-drive B --source-write-protected
```

No USB scan is required. `recover` defaults to **ibm.1440**, not automatic format discovery; use `--profile ibm.720` for verified 720 KB media. Confirm disk number and physical protection before starting. Default passes are Fast (2 revolutions, 0 retries), Normal (3, 2), Recovery (5, 3), Detective (8, 5). It stops on a complete reported map, two consecutive non-improving passes, four passes, or the 600-second acquisition ceiling (downstream file processing has separate limits). Later passes read problem cylinders plus two clean control cylinders, both heads, with the fixed profile. No routine drive swapping or speculative byte repair.

Raw captures and decodes remain separately hashed. A durable per-disk journal in `Flux/Recovery` reuses a saved whole-disk decode when available, resumes saved raw evidence after decode failure, and verifies completed artifacts on repeat invocation instead of reading again. A per-project/disk lock blocks duplicate jobs for that disk. CLI `scan`, `recover`, `capture`, and `info` share an OS-held reservation at `%APPDATA%\FluxVault\.fluxvault-greaseweazle.lock`, across projects using that settings location. It releases when the owning process exits; the remaining lock file is harmless. Direct `gw.exe`, separate Windows users/settings locations, and library consumers are outside this CLI reservation. Matching controls help detect disk swaps but cannot prove identity.

### Guided Greaseweazle-only batch

```powershell
fluxvault disk select 1
fluxvault greaseweazle scan --gw-drive B --source-write-protected --count 10
```

Use the project's next unscanned number (or a fresh project). Each swap requires the displayed number (`004` or `4`); bare `READ`, a different number, or arbitrary input starts no read. Legacy `READ 004` also works. `QUIT`/`Q` or end-of-input stops feeding. `--count N` caps results finalized this invocation, including resumed numbering commits. Plain `scan` reuses saved selector/profile/map/policy/end target; fresh projects use B, ibm.1440 and the normal policy. `--acquisition-only` skips downstream work. Every numbered confirmation includes the prompt's physical write-protect check.

Each disk uses the existing bounded recovery service. A verified terminal result—including partial/unrecoverable-within-policy—advances the project number. Operation failure keeps it selected. Numbered custody, pending result and completion history are atomically recorded in the internal project-root `.fluxvault-gw-scan.json`; an OS-held `.fluxvault-gw-scan.lock` prevents duplicate project scans. Control files and partial metadata commits are excluded from packages. Do not manually edit the journal or change disk selection from another session.

On restart, a pending result is verified against the committed single-disk journal, raw/decoded evidence and published hashes before reconciling the cursor. A cursor at either the pending disk or its next number is accepted; it advances only once without a physical read. An interrupted job with no recorded result asks for the same numbered custody confirmation, then lets the single-disk journal resume/reuse evidence. Different pending settings, changed published bytes, invalid identities or cursor disagreement refuse further reads.

After feeding finishes, saved-image extraction/conversion/audit/workbook processing runs once automatically; `--acquisition-only` skips it. This serial tail is not concurrent background scheduling. A failed tail retains all acquisition evidence and numbering; rerun `process` or quit a resumed scan to retry it. Empty new scans do not launch downstream work. JSON stdout contains session results, `total_scanned`, `resumed_advances`, `pending_disk`, next number and processing details; prompts/progress go to stderr. Exit 3 indicates partial history, a pending job, or downstream attention, not an automatic delivery certificate.

Mock-executable tests cover numbered confirmations, two-disk acquisition, disconnected-board failure, bounded partial recovery, restart on both sides of the numbering commit, changed-image refusal and cross-project contention. A separately running scan is terminated in the lock test; a new session then acquires the released reservation without starting a host tool. The guided batch itself still needs multi-disk hardware acceptance.

### Measured pilot runs

For the first twenty numbered customer disks, see [PILOT_20.md](PILOT_20.md). `scan --profile-map FILE` accepts a regular workstation JSON file (maximum 64 KiB) of the form `{"schema_version":1,"profiles":[{"disk":9,"profile":"ibm.720"}]}`. Entries override the default/`--profile` for their disk only; formats must be exactly `ibm.1440` or `ibm.720`. Duplicate/invalid numbers, unknown fields and unsupported schemas/formats are refused. The resolved map is persisted with pending scan settings and recorded in telemetry, along with each read's chosen profile; changing it while a disk is pending is refused. Older journals without a map retain their existing default-profile behavior. This supports known mixed batches, not automatic physical format detection.

Normal GW scans preflight 7-Zip and LibreOffice before displaying a custody prompt or starting acquisition. Failures preserve the cursor and do not create a pending scan job. `--acquisition-only` explicitly skips these downstream requirements. Tools are rechecked when processing actually runs, so a later failure still retains all saved evidence. `policies/pilot-short.json` provides an opt-in three-pass, 180-second acquisition policy for the small pilot; default recovery limits are unchanged.

`greaseweazle scan --last-disk 136 --gw-drive B --source-write-protected` stops at an absolute numbered endpoint, including after restarts. `--count` remains a separate per-invocation cap; both can be used together. The end target is persisted with pending settings, and changing it for a pending job is refused. Existing scan journals without the new optional field remain readable.

Each scan writes uniquely named, synced local JSONL events under `Logs/Benchmark` and creates new JSON/CSV snapshots under `Reports/Benchmark` at normal session completion. `benchmark report [--json]` re-exports saved telemetry without host/drive access. It includes configurations/executable fingerprint, confirmation waits, recovery/verification times, mean/median/p95 times, explicit partial/conflicting-sector outcomes, reported physical-read counts, failures, downstream summaries and incomplete sessions. Unique disk counts survive resumed commits. Exports contain no customer file contents and are excluded from customer packages; logs may contain private paths.

The feed-only 136-disk projection is an observed-sample extrapolation, excluding failures, downstream work, inter-session downtime and some persistence overhead—not a six-hour production guarantee. The simulated 136-disk scan service test covers partial outcomes, an acquisition failure, persisted resume, numbering through 136 and automatic stopping before 137. No hardware throughput is inferred from mock tests. See [PILOT_136.md](PILOT_136.md) for setup, acceptance order and the data collection recipe.

The final numbered image in `Images` has compatible acquisition metadata/logs and a hashed provenance map. Agreeing independent captures are labeled corroborated; sectors reported good in only one capture remain explicitly lower confidence. Conflicting/unreadable sectors are zero-filled and marked bad, never silently chosen or guessed. This is an immutable derived image, not an untouched raw capture or customer-delivery certification.

By default `recover` runs the project-wide `process` chain afterward (recovery/extraction, conversion, audit, workbook). `--acquisition-only` skips it. Partial FAT12 images now automatically get native readable-chain extraction when their saved evidence establishes a complete sector map; unknown maps or unsupported layouts remain exceptions. Repeat invocation can retry downstream processing with zero new physical reads, even with the board disconnected. Completed jobs do not start a new recovery budget; `capture` remains the expert path for a deliberately new attempt.

`--policy C:\path\to\policy.json` accepts this structure:

```json
{
  "passes": [
    {"name": "Fast", "revolutions": 2, "retries": 0},
    {"name": "Recovery", "revolutions": 5, "retries": 3}
  ],
  "max_seconds": 600,
  "no_improvement_limit": 2
}
```

Limits: 1–8 passes, 30–1800 seconds, 1–3 non-improving passes, 1–10 revolutions and 0–10 retries per pass. Resume requires the same disk, policy, profile, and selector. Failed/interrupted captures retain numbered partial evidence; only completed raw artifacts can be decoded without a new physical read.

`greaseweazle decode N` uses the latest completed SCP attempt by default, or `--capture-attempt N`, and verifies the source hash before running file-to-file `gw convert`. Derived images are kept separately under `Flux/Derived`, with their own hash and provenance, and are **not promoted into Images or treated as clean sectors**. The decoder records any `Found X sectors of Y` summary reported by Greaseweazle. For standard IBM 80-cylinder profiles, it records exact missing LBAs only if the entire reported grid is present and consistent; otherwise the map stays unknown. This is still Greaseweazle's report, not independent verification of each image sector. `greaseweazle status N` re-hashes saved raw and derived evidence, highlights missing/changed files, and shows reported sector counts/map availability without needing the board or host tool. Both decode and status return exit code 3 because sector-quality integration and delivery certification remain open. No physical drive is accessed for decode or status. `gw read --format` without `--raw` can regenerate flux rather than preserving what the disk emitted; FluxVault always pairs them for raw SCP captures, as documented by the [Greaseweazle image-type guide](https://github.com/keirf/greaseweazle/wiki/Supported-Image-Types).

`greaseweazle compare N` is an offline, read-only comparison of the best saved USB attempt and latest decoded flux image. It rechecks both image hashes and matching geometry, requires a complete Greaseweazle sector map, and reports sectors good in both but byte-disagreeing, USB-bad/flux-reported-good donor candidates, USB-only-good sectors, and still-unresolved sectors. Donor candidates are **not** automatically merged or certified; Greaseweazle's map is still vendor-reported evidence. A missing/inconsistent map, changed hash, unsafe path, or geometry mismatch stops comparison. No floppy drive or external tool is accessed.

`greaseweazle consensus N` compares the latest decodes from the two latest **distinct raw capture attempts**. It requires the same format, complete sector maps, intact raw and decoded hashes, and reports exact LBAs that agree byte-for-byte, conflict despite both being reported good, appear good in only one pass, or remain bad in both. Two decodes of one SCP do not count as independent captures. This is an evidence check, not a promoted composite or customer-delivery certification; it never accesses a floppy.

`greaseweazle plan N` combines the best saved USB attempt with decodes from the two latest distinct raw captures. It verifies paths, hashes, geometry, and complete reported sector maps before identifying USB-bad sectors where both flux images report a read and agree byte-for-byte. It separately lists one-flux-only sectors, unresolved sectors, and any USB/flux or flux/flux byte conflicts. Matching control sectors provide useful evidence but are not proof of physical disk identity. The command is offline and read-only, always returns attention code 3, and **does not create or promote a composite**.

Failed or interrupted offline decodes retain a numbered `.partial.json` attempt record (and any partial image). `greaseweazle status N` shows these as needing attention, and the next decode uses a new number instead of overwriting the failed attempt.

Office/PDF reuse requires both a matching saved source hash and a matching saved output hash, even after a restart. An older valid-looking output with no saved binding is preserved and reported as an issue rather than silently claimed as current conversion evidence. `conversion issues` reloads saved issues on every invocation.

### Native file recovery from damaged saved images

```powershell
fluxvault recovery extract 7
```

This command needs no physical disk or external extraction tool. `extract disk N`, `extract all`, and `process` also try the same native fallback automatically for partial images, failed extraction/listing, or zero-file extraction. Those wider commands still require their normal external tools.

Native recovery requires a hash-matching image, a recognized completed acquisition log/map, readable boot-sector geometry, and a supported unpartitioned FAT12 volume (512-byte sectors, at most 4 MiB). It traverses intact directory entries and FAT chains, including fragmented files and nested directories. A readable FAT copy can supply an entry missing from the other; readable copies that disagree are refused. Missing directory sectors, chain cycles, cross-links, size mismatches and unreadable file content are recorded rather than guessed.

Current files are published separately under `Extracted/NNN/attempt_NNN_native_v2` (or `legacy_native_v2`), preserving earlier extraction/manual recovery. Provenance is in `Recovery/NNN/attempt_NNN_fat12_v2.json`, with source hash, per-file hashes, content/metadata LBAs, allocation links, skipped entries, directory gaps and name evidence. First-generation unversioned folders/reports remain intact; running the recovery/extraction command can publish the improved generation without a new acquisition. Its inventory/provenance is rechecked before reuse/delivery mirroring. Existing differing files/reports are not silently replaced.

Intact VFAT long names retain their Unicode spelling only after full slot sequence, short-alias checksum, type/cluster, UTF-16, length, padding and safe-path validation. Their raw UTF-16 units and directory-entry offsets are recorded. Missing/damaged/unsafe name entries retain a safe 8.3 alias and a fallback reason; ASCII lowercase flags are honored, while unknown OEM bytes remain escaped. File/ancestor name and alias collisions are conservatively refused. This does **not** reconstruct missing name bytes, deleted/orphaned files or missing boot metadata, or carve files. Complete recovered file bytes are not a claim that the entire disk or customer job was recovered; single-capture confidence is not upgraded. `recovery extract` returns attention code **3**, including successful reuse. Batch recovery exceptions appear in `RecoveryExceptions.txt`; `BrokenForDMDE.txt` remains a compatibility filename, not a mandatory manual-DMDE instruction.

Validated `System Volume Information`/`$RECYCLE.BIN` directory names remain available in forensic extraction/inventory but are excluded from new delivery mirrors and packages. An unvalidated `SYSTEM~1` alias is not guessed to be an OS folder. Existing older delivery copies are preserved; automatic provenance-checked retirement/quarantine of obsolete mirrors remains a planned task.

Saved-image validation on 2026-10-06 recovered **22 forensic files / 1,208,710 bytes** from WinWord 1, preserved the original image SHA-256, and retained partial audit status for unreadable LBA 24. Generation 2 validated two long names and matched all 22 paths and hashes against independent 7-Zip extraction. A fresh `process` run produced 21 installer delivery originals, excluding one 76-byte Windows metadata file. Verified reuse and non-overwriting generation upgrade passed. There were no new physical reads and no Office conversion candidates on that installer disk. This is not a full corrupted-filesystem recovery benchmark.

`finalize --destination PATH` combines saved-image processing and package verification in one command. It requires at least one image and an existing destination outside the project. If recovery, conversion, or audit still needs attention, it reports that status and does **not** create a package. A successfully verified archival ZIP is still not a certification that every original customer byte was recovered.

The CLI has guided USB and Greaseweazle disk-change loops and bounded single-disk Greaseweazle-only recovery. Automatic format discovery, full damaged-filesystem extraction, background production scheduling, and concurrent two-drive workflow remain tracked in [TODO.md](TODO.md).
