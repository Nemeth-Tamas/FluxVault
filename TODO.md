# FluxVault — TODO

> CLI-first floppy archival, forensic imaging, recovery, conversion, audit, and customer-delivery suite. Work from a project folder in PowerShell using `fluxvault` commands; there is no desktop GUI.
>
> **Primary rule:** Source floppy media is read-only. FluxVault may write images, logs, extracted files, reports, and packages to the workstation, but it must never intentionally write to a customer floppy.
>
> **Product target:** FluxVault is an automated archival appliance, not a collection of expert-only recovery tools. Except for physically inserting, removing, or moving a floppy between drives, the normal operator workflow should require no recovery decisions, no manual DMDE work, no hand-edited spreadsheets, no manual extraction, and no manual report/package assembly. The intended production loop is: **run `fluxvault production start` in the project folder -> insert floppy -> confirm the swap cue -> repeat**. This production command is planned, not implemented yet.
>
> **Throughput target:** With one USB floppy drive and one Greaseweazle-connected drive operating concurrently on different disks, a 136-disk mixed-condition job—including automatic verification, escalation, extraction, conversion, audit, and ordinary recovery passes—should be achievable within one operator afternoon (target: no more than roughly 6 hours of attended wall-clock time, excluding genuinely pathological media that must continue unattended or be reported as unrecoverable).

> **Greaseweazle-only priority (2026-10-06):** USB is optional. On the working Mitsumi drive (straight cable, selector B), start fast, decode preserved raw evidence, and escalate only problem areas within time/media-stress limits. Routine drive/ribbon swapping is not part of the workflow. `greaseweazle recover N` implements bounded single-disk recovery; `greaseweazle scan` adds numbered/Enter swap confirmations, resumable numbering and continuous saved-image processing. Standard IBM format discovery and raw-only ambiguity continuation are implemented; additional decoders, deeper damaged-filesystem recovery and live production acceptance remain open. See `CHAT_TO_CHAT_GREASEWEAZLE.md` for the hardware handoff and `GREASEWEAZLE_PREFLIGHT.md` for live results.

> **Native recovery checkpoint (2026-10-06):** Saved partial FAT12 images now automatically yield independently intact files through native readable-chain extraction. The saved WinWord 1 capture produced 22 forensic files / 1,208,710 bytes, with unchanged source hash and the unreadable sector still flagged. Native generation 2 validates long names, preserves ASCII short-name case flags, and excludes identified Windows metadata from new delivery plans (21 installer originals). This replaces manual extraction for that bounded case, not full DMDE capability: missing boot/directory reconstruction, deleted/orphaned chains, carving, automatic format discovery and production scheduling remain open.

> **First 20-disk pilot completed (2026-10-06):** Customer 001–020 saved with 17 clean results and three partials (005/012/017, one missing sector each, no conflicting sectors). All 20 reached extraction; all 172 conversion jobs succeeded. The operator approves number-only scanning and requests much more visible swap cues. Next priorities: terminal visibility, saved/configurable scan conversion workers with balanced scheduling, and lossless background raw-capture compression. See `PILOT_20.md` for measured timing/storage and the extraction caveats; this is not yet full script/DMDE yield equivalence or customer-delivery certification.

> **Offline preparation checkpoint:** Visible/colored cues, balanced configurable conversion workers and Enter-only confirmations are shipped. New scans now add bounded IBM format discovery and verified background packing. A copy of all 26 captures shrank 76.42%, with 20/20 disks hash-healthy and packed 007 decoding identically. Original pilot raw files remain unchanged. See `PROGRESS.md`; full script/DMDE yield equivalence and production acceptance remain separate targets.

> **Damaged-filesystem starter:** Native generation 3 can infer an explicitly warned standard 720 KB/1.44 MB layout when boot metadata is missing, only with corroborating readable matching FATs/root file chains. It never synthesizes boot/file bytes or certifies inferred geometry. Skipped files retain exact holes/unmapped tails. Four saved pilot images retain 34 payload hashes under simulated boot loss; 017 is conservatively refused without its readable BPB. Historical 009's live JPEG has 4,608 unreadable bytes, so no complete payload is exported. See `DAMAGED_FILESYSTEM_RECOVERY.md`.

> **Continuous saved-file processing:** New scans enqueue verified acquisitions for coalesced background extraction/recovery/conversion/audit/workbook work; swap cues remain unobscured. Durable jobs, source verification, whole-project writer ownership, short image-publication snapshots, conservative CPU allowance, status/resume and final reconciliation are implemented. Old journals retain tail mode. See `BACKGROUND_PROCESSING.md`; live scan overlap and full 136-disk acceptance remain distinct validation gates.

## 0. Development contract / project rules

- [x] Rust stable, Windows-first application.
- [x] CLI-only executable; remove the desktop window, file dialogs, GUI session state, and their dependencies without removing the shared workflow services.
- [x] Keep long operations observable and interruptible from the terminal; physical reads, hashing, extraction, conversion, packaging, and Greaseweazle processes report progress on stderr and preserve logs.
- [x] Edit files locally and verify changes before pushing.
- [x] Keep the GitHub repository as the source of truth with logical, tested checkpoints.
- [x] Git workflow always uses `git add .`.
- [x] Do not use selective `git add <file>` instructions.
- [x] Prefer small modules with explicit responsibilities over a giant `main.rs`.
- [x] Errors shown to the operator must preserve the underlying technical detail in logs.

### Automation-first operator contract

- [ ] The default CLI workflow must be a guided production queue, not a chain of expert-only commands.
- [ ] The operator's only routine responsibilities are placing/removing disks, moving a disk from USB to Greaseweazle when prompted, and optionally entering a physical label or note.
- [ ] FluxVault automatically chooses retries, read direction, composite inputs, extraction strategy, recovery escalation, conversion, audit, and packaging policy from recorded evidence.
- [ ] Expert subcommands/flags remain available, but normal jobs must not require understanding sectors, FAT, DMDE, flux, profiles, hashes, or conversion filters.
- [ ] Every automatic decision records its evidence, confidence, limits, and provenance so automation never hides guessing or fabricates recovered data.
- [ ] A disk may finish as **verified**, **partially recovered**, or **unrecoverable within policy**; the program must not block the entire batch waiting for manual repair.
- [ ] All stages are resumable after application restart or workstation failure without repeating completed evidence-preserving work.
- [ ] No CLI prompt should ask the operator to make a technical choice the program can derive safely.

## 1. Safety invariants — must exist before real media testing

- [x] Create a central `MediaSafetyPolicy` / equivalent that marks all physical-floppy operations as READ ONLY.
- [x] Windows USB-floppy backend opens `\\.\A:` (or selected drive) with read access only; never request write access.
- [x] No code path may call a filesystem write operation against the floppy drive letter.
- [x] Greaseweazle integration exposes acquisition/info/convert operations only.
- [x] Never expose or invoke `gw write`, erase, clean, or another destructive Greaseweazle operation.
- [x] Print **READ ONLY** on physical-drive acquisition and report read-only access in machine-readable results.
- [x] Recommend the physical write-protect tab for customer disks when available.
- [x] Query Windows disk writability without attempting a write; refuse a full USB image if protection is not positively reported.
- [ ] Validate the current USB floppy drive's write-protect reporting with a **known-good disposable** floppy only. It reported `writable` with the physical tab open, and Windows-created filesystem metadata appeared between archived and fresh customer-disk images. On damaged disposable 001, a deliberate one-byte marker write to a previously readable sector failed with device I/O error 1117; the immediate read failed, but after eject/reinsert the sector again matched its archived SHA-256 exactly. This confirms no persistent change to that sector, **not** that the adapter enforces write protection. On 2026-09-26, the same drive reported `protected` with customer 007 and completed a read-only CLI image; one sector was unreadable and the other 2,879 matched the earlier clean image exactly. This read validates acquisition, not the adapter's physical write-blocking behavior. Never test writes on customer media; finish independent validation using a known-good disposable floppy or a verified hardware write blocker/GW setup.
- [x] Keep a command/audit log for every external tool invocation.
- [x] Never silently overwrite a previous acquisition/recovery attempt.

## 2. Project data model

- [x] Define a FluxVault project root while remaining compatible with the current archive layout during migration.
- [x] Recognize/use the existing directories where present: `Images`, `Logs`, `Extracted`, `Converted`, `Recovery`, `Reports`.
- [x] Add `Flux` (or equivalent) for raw Greaseweazle captures.
- [ ] Add a small FluxVault project metadata file (`project.json` or similar) containing project name, created time, operator settings, next floppy number, and tool paths/versions.
- [ ] Model each floppy as a stable record with zero-padded number (`001`, `002`, ...), label/notes, acquisition attempts, current preferred image, extraction state, recovery state, conversion state, and audit state.
- [x] Model acquisition attempts as immutable records: source backend, timestamp, geometry/format, output artifacts, hashes, bad-sector map, status, and log path.
- [ ] Allow one attempt to be marked **preferred/current** without deleting older attempts.
- [ ] Preserve enough provenance to answer: “Which read/pass produced this sector/file?”
- [ ] Import an existing script-created archive as a project without forcing re-imaging.

## 3. CLI operator experience

- [x] No-argument `fluxvault` shows help instead of opening a window.
- [x] Folder-first project discovery plus `init`, `status`, disk, drive, recovery, extraction, conversion, audit, report, package, and tool commands.
- [x] Saved command logs and stderr progress replace the window's status/operator-log panel.
- [x] **Unmistakable swap/action banners.** Separate physical completion from downstream processing, prominently display `DONE 004 / REMOVE 004 / INSERT 005`, and explicitly say when waiting for the operator rather than silently appearing busy.
  - [x] Add semantic terminal colors: green clean completion, red partial-saved swaps with explicit safe-to-proceed text, amber warnings/raw-only exceptions, cyan next physical action, red operation failure without number advancement. Always include plain-text labels and disk/station identity; never rely on color, special glyphs, or animation alone.
  - [x] Show temporary interactive read/recovery loading progress with ASCII heartbeat, elapsed time and unique host-reported track visitation, resetting targeted ranges without inventing sector-yield percentages. Clear before diagnostics and success/error swap cues; no animation in redirected stderr/JSON or dumb terminals. Test cleanup, quiet heartbeat, duplicates/range bounds and forced-color redirection.
  - [x] Support automatic terminal color detection, explicit color override, and `NO_COLOR`; readable ASCII/monochrome fallback on Windows, no automatic ANSI escapes in redirected logs or JSON/stdout. `--color always` explicitly forces stderr decoration; unit/executable tests cover plain/color/JSON output modes.
  - [ ] Show concise reading/decoding/processing/waiting status with elapsed time; preserve a clear next-action cue when concurrent worker messages arrive. Optional configurable audible cues may supplement, not replace, the banner.
  - [x] At the configured endpoint print `BATCH FINISHED / REMOVE 020`, then downstream progress/results; distinguish a persisted next cursor of 021 from an instruction to insert 021. Show session totals and whole-project totals separately.
- [x] Add explicit testing convenience `scan --no-verify`: Enter confirms the displayed disk, with a prominent warning at every custody prompt. Skip label typing only, never source read-only access, evidence/hash verification, numbering/resume safeguards or per-disk confirmation. Record mode in telemetry; do not persist the shortcut into future scans. EOF never starts a read, wrong explicit numbers remain refused, and default blank-input refusal is retained.
- [x] `greaseweazle preview` preserves the former safe-command mock/preview without touching hardware.
- [x] `greaseweazle info` exposes the audited read-only device/firmware query through the CLI; live V4.1/firmware 1.6/Mitsumi-B operation validated on 2026-10-05.
- [ ] Make the default command path much shorter: one production command runs the full safe chain with plain-language status and next physical action.
- [ ] Print concise station-specific prompts: “Insert floppy #NNN in USB”, “Move floppy #NNN to Greaseweazle”, or “Archive floppy #NNN and insert #NNN+1”.
- [ ] Add safe pause/resume/cancel semantics and a persisted job queue; a terminal closing must not silently lose completed evidence.
- [ ] `status` should show both stations, queued escalation, throughput, estimated time, current operation, and the next physical action.
- [ ] Unavailable hardware/tools must be reported with an actionable reason; never imply a planned capability already works.

## 4. External-tool discovery

- [x] Tool manager detects/configures:
  - [x] 7-Zip (`7z.exe` / `7zz.exe` / `7za.exe`).
  - [x] LibreOffice (`soffice.com` preferred, `soffice.exe` fallback).
  - [x] Greaseweazle host tools (`gw.exe`) when installed later.
- [x] Store operator-selected paths in settings.
- [x] Show detected version and health check with `tools check`.
- [x] Provide a `tools check` command.
- [x] Capture stdout/stderr and exit code for every external process.
- [x] Kill the full LibreOffice process tree on Windows timeout; verify with a disposable parent/child process test and report a termination failure instead of falsely claiming success.

## 5. USB floppy acquisition MVP — **first working milestone**

This is the first “we can actually use FluxVault on customer media” target. It should replace the manual `FloppyArchiver_v1.5_manual.ps1` workflow before we chase fancy recovery features.

- [x] Enumerate/select floppy drives on Windows; A: must work with the current USB floppy reader.
- [x] Current fallback: manual insertion/removal confirmation without depending on flaky automatic USB-floppy media detection.
- [x] Probe media safely by opening the raw device read-only, requesting geometry, and performing a tiny real read.
- [x] Read and display geometry: cylinders, heads, sectors/track, bytes/sector, total sectors, total bytes.
- [x] Fast path: read one full track at a time.
- [x] On track read failure, fall back to sector-by-sector reads for that track.
- [x] Configurable retry count for failed sector reads (initial default matching current tooling: 2 retries after first attempt).
- [x] Log every retry and recovery-after-retry event.
- [x] Zero-fill sectors that remain unreadable **only in the derived sector image**, while separately recording their exact LBA/CHS status so zeros are never mistaken for valid recovered data.
- [x] Write to `NNN_attempt_NNN.partial.img` first.
- [x] Validate exact expected image size before promotion.
- [x] Atomically promote completed output to the attempt image; do not leave a misleading “complete” file after a fatal error.
- [x] Compute SHA-256 of completed image.
- [x] Save structured acquisition metadata plus a human-readable log.
- [x] Show a live 80x2-ish track/head/sector heatmap: unread, good, retry-recovered, bad.
- [x] End state clearly reports `OK`, `PARTIAL`, or `FAILED` and exact bad-sector count.
- [x] Offer **Next floppy** while preserving media-change confirmation.
- [ ] Add audible completion/error cues optionally (configurable).
- [ ] Test against several known-good disks and several damaged disks from the current batch.
- [ ] Add a continuous production mode that automatically runs acquisition, hashes, triage, extraction, and audit after one **Disk inserted** confirmation.
- [ ] Detect stable media removal/insertion automatically where the hardware permits, while retaining one-button confirmation as a reliable fallback.
- [ ] Automatically advance the disk number only after the current disk has durable image/log/metadata evidence and a recorded next action.
- [ ] Automatically choose bounded retry count/direction from read results; expose the policy rather than asking the operator per disk.
- [ ] Automatically queue non-clean USB results for Greaseweazle instead of requiring the operator to inspect a recovery page.

### MVP acceptance test

- [ ] Insert a known-good 1.44 MB floppy -> FluxVault creates a 1,474,560-byte image, zero bad sectors, SHA-256, log, and project record without writing to source media.
- [x] Insert a known-bad floppy -> FluxVault completes a correctly sized image where possible, identifies exact unreadable sectors, preserves the partial status, and routes the disk to Recovery.
- [x] Re-run the same floppy -> creates/preserves a new attempt instead of destroying the previous evidence.

## 6. Existing archive/log compatibility

- [x] Parse current `FloppyArchiver` logs (`BEGIN`, `GEOMETRY`, retries, `BAD_SECTOR`, `SHA256`, `END`).
- [x] Parse current DMDE Copy Sectors logs using the same **multi-pass/latest-sector-state-wins** rule as the PowerShell tooling.
- [x] Recognize forward and reverse DMDE passes.
- [x] Preserve statuses for unfinished logs as **IN PROGRESS**, not “broken”.
- [x] Exact `NNN.log` must outrank auxiliary `NNN_scan.log`, retry-note logs, etc.
- [x] Import existing `.bin`, `.img`, `.ima` images and ignore `.partial.*` files as completed acquisitions.
- [ ] Import existing hashes and current archive index where possible.
- [ ] Display legacy/manual recovery state without requiring the old Excel workbook.

## 7. Extraction pipeline

Initially reproduce the proven script workflow; we can replace pieces with native Rust later where it actually helps.

- [x] Clean image -> test FAT readability with 7-Zip before extraction.
- [x] Capture detailed 7-Zip listing as audit material.
- [x] Extract into a temporary working directory first.
- [x] Only replace/promote an automatic extraction after the new extraction fully succeeds.
- [x] Write per-floppy file inventory with relative path, bytes, modified time, attributes, and SHA-256 where appropriate.
- [x] Record the source image SHA-256 marker so unchanged images do not need needless re-extraction.
- [x] Re-hash all managed extracted files against the saved inventory before reusing an extraction; reject missing, altered, or extra files.
- [x] Detect an operator-created recovery folder without the auto-extraction marker as **manual recovery present**.
- [x] Preserve the current concept of an immutable first recovery backup (`Recovery/NNN/pass1` or equivalent).
- [x] Build/update a project-wide recovered-file manifest.
- [x] Batch extraction selects the best known attempt rather than blindly using the latest; managed manifest rows use the exact extraction source hash and re-verify file integrity.
- [x] Run project-wide batch extraction/recovery routing and emit script-compatible summary/review lists.
- [x] Automatically try native FAT12 readable-chain extraction for partial images, failed 7-Zip extraction/listing, or zero-file results; retain partial status, immutable backups, and legacy report compatibility.
- [x] Publish native files into separate `attempt_NNN_native`/`legacy_native` managed folders with source/file/provenance hashes; verify unchanged files and reports before reuse or delivery mirroring, without replacing existing extraction/manual recovery.
  - [x] Preserve earlier native generations during engine upgrades: publish current output into `attempt_NNN_native_v2`/`legacy_native_v2`, with a separate versioned report; prefer the latest engine generation for the same numbered acquisition and verify reuse.
- [x] Emit `RecoveryExceptions.txt` alongside legacy `BrokenForDMDE.txt`; unresolved cases are exceptions, not instructions to do manual DMDE as the default workflow.
- [x] Route these cases to Recovery instead of pretending success:
  - [x] non-clean image / unreadable sectors;
  - [x] missing/unrecognized acquisition log;
  - [x] unfinished acquisition log is preserved separately as **IN PROGRESS**;
  - [x] FAT listing failure;
  - [x] extraction failure;
  - [x] apparently readable image with zero recovered files when operator review is warranted.
- [ ] Automatically run extraction immediately after an eligible acquisition or newly derived preferred image; no separate Files-page action in production mode.
  - [x] Single-disk `greaseweazle recover N` dispatches the existing project-wide extraction/conversion/audit/workbook chain by default; partial images still follow conservative extraction eligibility rules.
  - [x] The retired GUI queued image-only extraction after each clean USB acquisition; the CLI retains `extract disk N`/`extract all`, but automatic nonblocking post-scan extraction remains a production-scheduler task.
  - [x] Add a one-button offline project pass that batches eligible extraction, conversion, evidence audit, and a Hungarian workbook without touching physical media.
- [ ] Automatically re-run extraction and file inventory whenever a better composite, decoded flux image, or reconstructed filesystem becomes preferred.
- [ ] Replace “operator review required” as the normal next step with a bounded automatic recovery plan; operator review is the final exception state only.
- [ ] Treat existing manual recovery folders/DMDE imports as legacy compatibility inputs, not as the intended future recovery workflow.

## 8. Automated recovery engine — pre-Greaseweazle

- [x] Recovery queue ordered by severity/attention state.
- [ ] Add a comprehensive `disk show N --details` view (and JSON equivalent) for current image, bad-sector list, source log, extraction result, prior attempts, automated decisions, and optional operator notes.
- [x] **Re-read with USB drive** action creates another immutable acquisition attempt.
- [x] Compare attempts sector-by-sector.
- [x] Reconstruct readable mirrored-FAT sectors into a separate derived image with per-sector provenance even when the disk has more than two bad sectors; other sectors remain unresolved and are never guessed.
- [x] Build an optional **best composite sector image** from multiple attempts, but only with a provenance map recording the source attempt for every replaced sector.
- [x] Reject composite sources whose recorded image hash changed or whose mutually readable sectors disagree; reject duplicate attempt IDs and unsafe source file types.
- [x] Add a read-only per-disk recovery plan (`fluxvault recovery plan`) that re-hashes saved attempts and ranks composite, mirrored-FAT, and physical reread/flux candidates without writing to source media.
- [x] The one-button offline `process` workflow automatically creates or verifies/reuses mirrored-FAT derived images where the saved-image plan finds redundant readable sectors; unresolved data stays explicitly partial.
- [x] The offline `process` workflow automatically attempts provenance-tracked composites when saved attempts have complementary readable sectors, reuses verified prior results, and declines conflicting captures without stopping other disks. It can then apply mirrored-FAT repair to a still-partial composite.
- [x] Persist each offline recovery decision and exception in `Reports/OfflineRecoveryDecisions.json` for audit/customer-package context; this is not a claim that unresolved sectors were recovered.
- [x] Never destroy original attempt images when creating a composite.
- [x] Legacy/fallback compatibility: allow import of a DMDE-recovered folder and DMDE log. This must not remain part of the intended normal workflow.
- [ ] Immediately re-run extraction/audit state after a new recovery result is imported.
- [ ] Hex/sector inspector for selected sectors with LBA + CHS + attempt provenance, available as an advanced diagnostic rather than a required workflow step.
- [ ] Build an automatic recovery policy engine that selects the next safe action from evidence and stops at configurable media-stress/time limits.
- [ ] Automatically combine all USB attempts, retry-recovered sectors, reconstructed FAT copies, and later flux-derived sector images into the best provenance-tracked derived image.
- [ ] Implement native FAT12 filesystem analysis/reconstruction sufficient to recover directory trees when 7-Zip cannot mount the image.
  - [x] Traverse intact root/subdirectory entries and fragmented FAT12 file chains from hash-checked saved images with complete acquisition maps; skip unreadable directory sectors and recover intact reachable files beyond those gaps.
  - [x] Consult every readable FAT copy; refuse disagreement, loops, invalid sizes, cross-linked ownership, unsafe/duplicate paths and unreadable file content rather than guessing or exporting zero-filled files.
  - [x] Record per-file data/metadata LBAs, cluster links, FAT-copy sources, raw short-name bytes and DOS timestamps; bind this report to the managed inventory and retain partial audit status.
  - [x] Recover intact VFAT UTF-16 long names only after sequence, short-alias checksum, slot type/cluster, length, padding, UTF-16 and safe-path validation; otherwise retain the recorded short alias with a reason. Record raw name units/entry offsets and name-sector provenance, including fragmented directories.
  - [x] Preserve FAT short-name ASCII lowercase flags without guessing OEM encoding; refuse long-name/short-alias and conservative Unicode case collisions in both files and ancestor directories.
  - [x] Add a missing-BPB standard-layout fallback for IBM 720 KB/1.44 MB images, requiring both complete readable matching FATs, valid allocation values, readable root metadata, a known end marker and a size-consistent live root-file chain; refuse contradictory surviving BPB fields and retain an explicit inference warning/metadata LBAs, not fabricated boot bytes or certified original geometry.
  - [x] Record skipped file candidates with logical byte-offset/LBA hole ranges (clipped to file size), allocation/name provenance and unmapped tail bytes; do not export invented payloads, guessed source sectors or hashes for incomplete files.
  - [x] Publish immutable native generation 3, preserving/reusing verified prior generations and operator files; expose inferred-layout warnings in both human CLI output and JSON, including verified reuse after restart.
  - [x] Validate missing-boot recovery in memory against saved 005/007/009/012 payloads, conservatively refuse 017's missing root end marker, and analyze copied historical 009 through the CLI with its completed DMDE sector map; source hashes remain unchanged.
  - [ ] Further reconstruct missing boot/directory metadata using sufficient cross-attempt evidence; add content validation and competing-layout analysis before inferred standard layouts can be certified. Missing directory entries, nonstandard layouts and root trees without the required live file anchor remain unresolved.
  - [x] Use validated long names to identify/exclude Windows OS metadata from new delivery plans/packages, while keeping forensic extraction/inventory; never guess that every `SYSTEM~1` alias is `System Volume Information`.
  - [ ] Automatically quarantine obsolete, hash-proven managed delivery mirrors when recovery naming/source generations improve; preserve changed/operator files and audit each retirement. Old delivery copies are currently left untouched, including earlier short-alias mirrors.
- [ ] Use both FAT copies, boot-sector/BPB evidence, root-directory entries, cluster chains, file sizes, and cross-attempt sector provenance to reconstruct damaged filesystems without arbitrary byte guessing.
- [ ] Add evidence-supported orphaned cluster-chain recovery, clearly labeling confidence and recovery method.
- [ ] Add explicit opt-in deleted-file recovery (e.g. `--include-deleted`) to saved-image processing; OFF by default, including new-project scan policies. Keep live/deleted/carved provenance separate, preserve originals, and never infer a complete file across unreadable bytes. DMDE was previously run with deleted recovery enabled; this is not a requirement for default customer delivery.
- [ ] Add signature-based file carving as an automatic fallback for unreconstructable filesystems, preserving raw offsets and labeling filenames/paths as reconstructed.
- [ ] Detect common document/archive/image signatures and validate carved outputs before including them in customer delivery.
- [ ] Try multiple evidence-ranked interpretations automatically and retain all non-destructive candidates; never require the operator to choose a sector manually.
- [ ] For disks with hundreds of bad sectors, recover every independently verifiable file/fragment possible, then produce a precise unrecoverable-range report instead of failing the entire disk.
- [ ] Automatically prefer a more complete recovery while retaining prior results and explaining why the preferred result changed.
- [ ] Replace reliance on interactive DMDE with native Rust recovery or another fully automatable, auditable read-only engine. DMDE may remain an optional compatibility/fallback adapter only if it can be automated legally and safely.

## 9. Greaseweazle integration — hardware-independent groundwork

Greaseweazle host tools are intentionally wrapped rather than reimplemented initially. Current upstream supports Windows `gw.exe`, raw-flux formats including SCP/KryoFlux, and a separate `gw convert` path, so we can build/test command generation and output parsing before the board arrives.

- [x] Create `GreaseweazleBackend` abstraction with a mock/no-hardware mode.
- [x] Detect `gw.exe`, run info/version command, and show device status.
  - [x] Match actual Windows host 1.23: obtain host version from `gw info` and parse normal info/progress on stderr as well as stdout. Mock executable reproduces that behavior instead of accepting unsupported `--version`.
- [x] Build commands as argument arrays, never shell-concatenated strings.
- [x] Unit-test command generation without hardware.
- [x] Add CLI raw-SCP capture and offline decode routes with mock-backed artifact tests, immutable attempt numbering, SHA-256 provenance, and source-hash refusal; do not call them live-validated before the board arrives.
- [x] Exercise the actual CLI end to end with an isolated mock `gw`: configured tool discovery, connected-board info, missing-board capture refusal before any artifact, two numbered raw captures, offline decodes, status, consensus, and command audit; no physical media involved.
- [x] `greaseweazle status N` verifies saved raw/derived hashes without hardware and distinguishes Greaseweazle's reported sector count from independently validated sector quality.
- [x] Conservatively parse exact missing LBAs from a complete, internally consistent IBM 80-cylinder `gw convert` grid; reject truncated/ambiguous grids and retain the result as vendor-reported evidence only.
- [x] Compare best hash-verified USB image with latest hash-verified decoded flux image offline, reporting byte conflicts and donor candidates without promoting either source.
- [x] Cross-check decodes from two distinct raw capture attempts offline, requiring intact hashes and complete sector maps and reporting exact byte conflicts without merging images.
- [x] Rank USB-bad donor candidates only when two distinct raw captures report the sector good and their bytes match; report one-pass, unresolved, and cross-source conflicts offline without changing images.
- [x] Retain failed/interrupted decode metadata and partial images as numbered evidence; never reuse their attempt number, and surface them in offline status.
- [ ] Validate Greaseweazle donor-sector bytes independently (including repeated flux decodes/captures where needed), then create an immutable provenance-tracked composite only when good-sector conflicts are resolved; never claim vendor-reported dots alone prove clean bytes.
- [x] Parse `gw` stderr/stdout incrementally into CLI progress/events.
- [x] Store full command, version, start/end time, exit status, and captured output for every run.
- [x] Bound and audit tool-health/version probes as well as `gw info`/read/convert, so a hung probe cannot stall the normal CLI path indefinitely.
- [x] Parse `gw info` device details rather than trusting exit code zero (which upstream can return for `Device: Not found`); surface not-found/unknown/bootloader states and block capture before reserving an attempt.

## 10. Greaseweazle raw-flux acquisition — after board arrives

- [x] Detect board and print device/firmware info; validate physical drive operation through a protected read. V4.1/Mitsumi selector B tested on 2026-10-05, host 1.23, firmware 1.6. This is not automatic drive-model detection or validation of the faulty NEC.
- [x] **Preservation capture defaults to true raw flux** (SCP), not regenerated “perfect” flux; physically tested on the Mitsumi setup.
- [x] Important guardrail: if `gw read --format=...` is used for a raw-flux file, pair it with `--raw`; otherwise Greaseweazle may regenerate flux and fill undecodable sectors rather than preserving the physical capture.
- [ ] Default recovery workflow: automatically capture raw flux once when USB triage escalates a disk, then perform as much decoding/re-decoding as possible from that preserved capture instead of repeatedly stressing fragile media.
- [x] Allow configurable revolutions for raw capture (SCP; 1–10 via CLI).
- [x] Preserve every raw acquisition as an immutable attempt with SHA-256, including numbered failed/partial evidence.
- [x] Derive sector images from raw captures using `gw convert --format=<profile>`; separate artifacts, never replacements for raw flux.
  - [x] Live raw capture/decode/status/independent-capture consensus passed with protected WinWord 1. Sparse targeted-capture grids parsed conservatively; unobserved cylinders stay unavailable.
- [x] Profiles initially required for this collection:
  - [x] IBM PC 1.44 MB / HD.
  - [x] IBM PC 720 KB / DD.
- [ ] Later expose other Greaseweazle disk definitions without hardcoding the whole universe into FluxVault.
- [ ] Track/head selection and step settings available as expert flags, not in the basic happy path.
- [x] Apply bounded single-disk automatic physical-read policies based on missing/conflicting sectors, elapsed time, revolutions, and prior improvement; stop rather than endlessly hammering media. Default 4 passes/600 seconds/2 consecutive non-improving passes; validated policies can tighten/change ceilings.
  - [x] `greaseweazle recover N` works without USB: Fast whole disk, then fixed-profile problem-cylinder rereads with clean control cylinders; preserves raw/decode attempts and publishes immutable compatible image/log/metadata plus per-sector confidence/provenance.
  - [x] Persist stages and completed result; resume saved raw decode without a new read, verify completed hashes, reject changed policy/settings, block duplicate per-project/disk jobs.
  - [x] Live targeted recovery retained 2,879/2,880 sectors, no byte conflicts, and stopped with persistent LBA 24 explicitly unreadable. Repeat invocation read no media; downstream backup/audit/workbook ran.
  - [ ] Add cross-project physical-device reservation and interruption tests at every journal/publish boundary before calling this a production scheduler.
    - [x] CLI `scan`/`recover`/`capture`/`info` reserve one Greaseweazle across projects sharing the per-user settings directory; mock child-process contention and termination/release tests pass. Direct host/library calls and separate users are outside this guard.
    - [x] Test guided-scan restart before/after the cursor commit, custody reconfirmation after acquisition failure, changed output refusal, and Windows metadata commit failure without truncating the old project file.
    - [ ] Exercise every acquisition-journal/publication interruption boundary and independently validate multi-disk hardware behavior.
    - [ ] Bind active host-process lifetime to the controller (for example Windows job-object supervision) and test forced termination during capture, not just idle custody; an exited parent reservation alone does not prove an orphaned host process has stopped.
- [ ] Automatically infer the first decode profile from USB geometry/image size and flux evidence, then try evidence-ranked alternative profiles without operator selection.
  - [x] New scans identify IBM 1.44 MB/720 KB from hash-verified whole-disk flux, readable BPB geometry and complete reported sector maps. Coherent >=98%-readable HD skips the alternative; otherwise try both offline. Without readable geometry require >=80% coverage versus <=5% in the alternative. Preserve candidate hashes/maps and immutable decision reports; refuse ambiguous/unsupported evidence without extra physical reads.
  - [x] Persist automatic/fixed mode and selected profile; resume with the selected geometry, bind aggregation/comparison to that profile rather than colliding HD/DD decode numbers, and record actual format in completion telemetry. Existing journals keep fixed defaults; explicit profiles/maps override automatic selection.
  - [x] Add offline `greaseweazle identify N` and a convert-only identify/decode backend that cannot probe a board for version discovery. Test DD selection, absent-board restart, explicit/auto resume guards, CLI defaults/packing and tamper refusal; validate saved 007-HD/009-DD.
  - [ ] Add nonstandard/severely damaged format discovery; represent ambiguity as a bounded queued exception and continue feeding rather than stopping custody at it.
    - [x] When both supported IBM candidate decodes finish but neither is convincing, preserve one verified whole-disk raw capture and a hash-bound format decision as a raw-only exception. Continue numbered feeding without inventing geometry, sector counts or a compatible image; retain attention status in scan/status/queue/audit/process/finalize and benchmark telemetry. Decoder failures still stop rather than masquerading as format ambiguity.
    - [x] Test raw-only packed-evidence restart without another hardware call, decision tamper refusal, unknown benchmark counts, two-disk continuation, and restart at the cursor boundary. Additional format decoders and raw-file extraction remain separate work.
  - [x] Initial CLI capture profile defaults from saved 1.44 MB/720 KB USB sector count; other/ambiguous formats require an explicit profile until flux-based inference exists.
- [ ] After flux capture, automatically decode, compare against USB attempts, build the best composite, retry extraction/recovery, and update audit state.
  - [x] GW-only single-disk job aggregates its independent raw captures with good-byte conflict refusal and explicit single-capture confidence, then calls `process`; full USB/GW composite integration and damaged-filesystem extraction remain open.
- [ ] Tell the operator exactly when to move a USB-problem disk into the Greaseweazle drive and when it can be removed; no flux expertise should be required.

### Raw-capture storage efficiency — requested after the 20-disk pilot

The pilot holds 26 immutable SCP captures (20 initial plus six targeted rereads), totaling 985.13 MiB; the whole working project is about 1.08 GiB. Flux preserves multi-revolution timing evidence that sector-only script images did not contain. Storage optimization must not discard that added evidence or substitute regenerated flux.

- [ ] Benchmark lossless capture compression on representative clean, damaged and targeted captures; report ratio, CPU/RAM use and elapsed time rather than promise a fixed saving. Compare equivalent folder/package contents with the old archive, not an uncompressed working project against a ZIP.
  - [x] Add image-only `storage benchmark N`: use the largest hash-verified complete capture (128 MiB bound), compare ZIP/Deflate levels 1/6, measure compression/decompression and require identical size/SHA-256; work in bounded memory without writing/removing evidence.
  - [x] Validate saved customer 007: 54,050,828 bytes -> 12,343,912 bytes at level 6 (77.16% saving), roughly 0.86 s compression / 0.07 s roundtrip verification on this workstation. This is one capture, not an aggregate guarantee; broaden samples and shared CPU/RAM budgeting before automatic compression.
  - [x] Build/hash-verify the real 20-disk archival package with all 26 captures: raw flux compresses from 985.13 to 232.24 MiB (76.42% saving); the complete 937-file ZIP is 266.19 MiB. Every ZIP member is checked against its manifest, and internal conversion history remains excluded. Workstation raw captures remain unchanged; transparent background packing/resume is still separate work.
- [x] Add one disk-streaming background post-capture packer with coalesced wakeups and durable tasks; enqueue complete verified captures only, never host-written partials. New scans use managed packed retention; old journals retain raw storage until explicitly changed. Worker chatter does not obscure swap cues. True host-stream compression remains optional future work.
- [x] Bind original/packed bytes, SHA-256, codec/version; independently decompress before atomic container/sidecar publication and reverify the published pair before raw retirement under exclusive ownership. `storage pack N` retains raw by default; `--retire-raw` is explicit. Acquisition metadata remains unchanged and original SCP bytes are recoverable from the container.
- [x] Support packed captures transparently in decode, flux status/hash checks, completed recovery resume, storage benchmarks and archival export. Use isolated verified temporary SCPs with shared-reader/exclusive-packer locks. Packages retain verified containers/bindings rather than excluding all ZIP evidence; logical original hashes remain stable.
- [ ] Persist queue state and test interruption, corrupt/truncated compressed files, disk-full, failed verification, repeated resume, cleanup and cross-process contention without evidence loss or duplicate ownership.
  - [x] Durable tasks and `storage resume` work offline; failed tasks/raw evidence remain, duplicate owners are refused. Tests cover ZIP-before-sidecar restart, repeated packing, corrupt container/member/hash refusal, reader/packer contention, isolated materializations/normal cleanup and recovery reuse. Windows preflights capacity. Real isolated copy: all 26 captures 1,032,982,606 -> 243,529,973 bytes in 27.07 s; 20/20 disks hash-healthy, packed 007 decoded identically, original pilot retains 26 raw files.
  - [ ] Add forced-process termination at every packing boundary, injected mid-write disk-full failure, orphan temporary cleanup and cross-process active-reader/packer soak coverage.
- [ ] Budget compression and conversion CPU/RAM/disk I/O together, prioritize acquisition, preflight free space and apply bounded backpressure; additional workers must not starve physical capture or flood the disk.
- [x] Document forensic raw/packed retention and archival export. Containers preserve exact physical flux, not regenerated or sector-only substitutes; archives retain original hash bindings. Raw retirement follows the managed policy or explicit offline flag and is reversible; the original customer pilot/archive remains unchanged.

## 11. Autonomous two-drive production workflow

Greaseweazle-only production is also a first-class mode; no USB scan is required. Build its guided disk-swap loop first around the single-disk recovery service, then add concurrent USB/GW scheduling. The working Mitsumi stays connected; alternate-drive comparison is an optional service action, not routine operator work.

- [ ] Add a guided Greaseweazle-only batch loop with automatic numbering, custody confirmation, concise swap cues, resume, and background downstream work.
  - [x] `greaseweazle scan --gw-drive B --source-write-protected [--count N]` prompts `READ NNN`/`QUIT`, runs bounded per-disk recovery, preserves partial results and advances durable project numbering only after verified publication.
  - [x] Persist pending custody/result and history; reconcile interrupted numbering exactly once without another physical read, reject mismatched settings/identities/cursor or changed evidence, and hold project/device reservations.
  - [x] Automatically run extraction/conversion/audit/workbook once after feeding ends; report tail failures without losing acquisitions. `--acquisition-only` skips the tail.
  - [x] Move saved-image extraction/recovery/conversion/audit/workbook to an isolated coalesced worker while feeding continues; new scans default to background, older journals retain tail, and the endpoint drains/reconciles all saved work.
  - [x] Persist image-hash-bound task identities/states atomically; reverify completed acquisition maps, resume interrupted/failed jobs, retain completed jobs for duplicate-safe restart, and bound failure attempts per worker session.
  - [x] Hold one project-wide workstation writer; refuse competing mutations, publish compatible acquisition metadata atomically and release the short artifact gate during long Office conversions. Probe ownership/stale status without hardware/tool access.
  - [x] Keep progress in a saved stage timeline/latest snapshot rather than overwriting swap prompts; expose `processing status` and offline `processing resume`, retain clear partial/failed outcomes and cap background workers to leave CPU capacity for acquisition.
  - [x] Coalesce 136 concurrent job arrivals into two processing passes in a deterministic soak; test duplicate enqueue, failed/interrupted restart, changed-source refusal/restoration, control bounds, stale status and competing CLI writers. Fix final-task drain races in processing and capture packing.
  - [x] Validate the full background chain on a separate copy of the real 20-disk pilot: two coalesced processing runs, all 172 conversion jobs successful, final 17 verified / 3 attention, and unchanged source-image hashes. Exercise the default scan-to-background CLI path using mock hardware and real saved-file tools; no physical-media access.
  - [ ] Live-test physical reads overlapping background processing and packed retention on clean/DD/damaged disks, then the 136-disk cohort; record contention, memory, final yield and complete-session throughput.
    - [x] Operator's 053–064 live batch validates clean/damaged HD acquisition with background file processing and managed packing: all 12 saved/extracted, 46 conversions OK, no background errors, final 10 verified / 2 attention. 059 retains five missing sectors; 062 improves from 44 to 14 after bounded rereads. Stage timestamps confirm processing during feeding; full DD/cohort/resource acceptance remains open.
- [x] Infer supported standard formats from saved-flux/BPB evidence in new scans; `recover --profile auto` enables the same path. Expert `recover` preserves its fixed-HD default and legacy journals retain their saved mode.

The target setup has two different drives working simultaneously on different floppies: the USB drive performs fast first-pass acquisition while the Greaseweazle drive processes disks automatically escalated from the USB queue. A single disk is never placed in both drives simultaneously; the scheduler tracks custody and tells the operator where each numbered disk goes next.

- [ ] Create a central CLI job scheduler with independent USB, Greaseweazle, CPU extraction/recovery, conversion, audit, and packaging worker queues.
- [ ] Run the USB and Greaseweazle physical drives concurrently on different disks without blocking hashing, extraction, conversion, or reporting workers.
- [ ] Automatically triage every USB result into **USB complete**, **USB re-read**, **move to Greaseweazle**, or **unrecoverable within USB policy**.
- [ ] Automatically prioritize the Greaseweazle queue by expected recovery value, severity, age, and whether the operator currently has the disk available.
- [ ] Maintain unambiguous disk identity/custody so results from two drives can never be attached to the wrong floppy number.
- [ ] Require a simple physical confirmation when moving a disk between stations, then verify geometry/fingerprint consistency before accepting the new attempt.
- [ ] Keep both drives busy whenever eligible work exists; CPU-heavy extraction/conversion must not stall physical acquisition.
- [ ] Allow the operator to continue feeding good disks into USB while Greaseweazle works on an earlier bad disk.
- [ ] Use audible cues and unmistakable terminal messages differentiated by station: **USB swap**, **move to Greaseweazle**, **Greaseweazle swap**, and **attention only if automation is exhausted**.
- [ ] Support pause/resume and clean shutdown while preserving every queue item and in-progress artifact safely.
- [ ] Estimate throughput and remaining batch time from observed read/retry/conversion durations.
- [ ] Add a production acceptance benchmark for the 136-disk reference job: complete ordinary dual-drive acquisition/recovery and downstream processing within a target six-hour operator session.
- [ ] Record operator touches per disk and target the theoretical minimum: initial insertion/removal plus one Greaseweazle transfer only for escalated disks.
- [ ] Provide an unattended tail mode so flux re-decodes, extraction, conversion, audit, and packaging can continue after the operator finishes feeding physical disks.

## 12. “Mini electron microscope the shit out of it” flux recovery diagnostics

- [ ] Track/head map for raw-flux capture quality.
- [ ] Per-track decoded sector summary: present, valid CRC, bad CRC, missing, duplicates/unusual IDs where available.
- [ ] Compare multiple revolutions/passes through text/JSON diagnostics and exportable data.
- [ ] Report weak/problematic regions and which decode attempt recovered each sector.
- [ ] Re-run decode from the same raw flux with alternate Greaseweazle profile/settings without touching the physical disk.
- [ ] Compare results from USB-sector reads versus Greaseweazle-derived sector images.
- [ ] Composite/reconstruction tools must retain provenance and never masquerade reconstructed bytes as an untouched original capture.
- [ ] Export a recovery note describing what was physical capture, decoded data, retry-recovered data, and reconstructed/composited data.

## 13. Legacy Office conversion pipeline

Reproduce `Convert-LegacyOffice_v4_Timeout_Audited.ps1` behavior inside the app workflow.

- [x] Mirror recovered originals into customer-facing `Converted` paths without changing the forensic source tree.
- [x] Remove DMDE artifact path segments from delivery paths (`$Noname`, `$Root`, raw-signature folders) while preserving forensic path mapping.
- [x] Resolve name collisions deterministically (`[recovered copy N]`).
- [x] Preserve recovery-method labels: normal filesystem, DMDE filesystem recovery, signature recovery.
  - [x] Label native FAT12 files distinctly as readable-chain recovery with unverified filesystem completeness in delivery-path reports and conversion jobs.
- [x] Support current source extensions/plans:
  - [x] Word-family -> DOCX + PDF: `.doc`, `.rtf`, `.wps`, `.wri`, `.wpd`, `.sdw`.
  - [x] Spreadsheet-family -> XLSX + PDF: `.xls`, `.xlw`, `.xlt`, `.wk1`, `.wk3`, `.wk4`, `.wks`, `.123`, `.wb1`, `.wb2`, `.wq1`, `.wq2`, `.sdc`.
  - [x] Presentation-family -> PPTX + PDF: `.ppt`, `.pps`, `.pot`, `.sdd`.
- [x] Bounded parallel Office conversion (four workers by default, CLI-configurable from 1 to 16), with isolated LibreOffice profiles, ordered reports, and serialized command audit records.
  - [x] Workers already claim the next job from one shared atomic queue rather than fixed per-worker batches; an idle worker immediately takes another job, while reports retain stable plan order.
- [x] Expose conversion worker count through `scan` and saved project settings, with expert override and a safe default; support 12 workers without editing policy files. `scan --conversion-workers 12` saves the count and forwards it to the automatic tail; old journals use four. Validate 1–16 before custody; the worker count is a downstream setting, not a physical recovery policy.
- [x] Add initial size/observed-duration-aware scheduling: interleave one estimated-long job with three estimated-short jobs in the shared queue; idle workers take the next job immediately, with exactly-once claims and stable report order. Use successful hash-matching prior durations, otherwise source size. Further CPU/RAM-aware adaptive scheduling remains open.
- [ ] Benchmark 4/8/12 conversion workers against saved pilot files, measuring throughput, peak memory, timeouts and output integrity; workers are concurrent jobs, not a guarantee of one CPU thread each or linear speedup.
  - [x] Exercise a fresh 172-job saved-image run at 12 workers: 164 OK/eight partial DOCX timeouts under the restricted desktop runner (process-tree termination reported access denied). Retry those eight at four workers outside that restriction restored 172 OK/zero exceptions, reusing 336 outputs. Subsequent canonical-path 12-worker processing reused all outputs with zero new LibreOffice conversions and restored the 17 verified/three attention evidence result. Keep four as the conservative default; do not infer twelve is a validated faster production preset.
- [ ] Stress-test mixed-size scheduling, same-stem/output collisions, concurrent CLI runs, retries, cancellation/restart and worker failure; require exactly-once claims/publication, isolated LibreOffice profiles, hash-bound reuse, serialized shared metadata/audit writes and complete deterministic reports.
  - [x] Test mixed-size scheduling with 12 workers, invalid/duplicate schedule refusal, bounded concurrency, deterministic report order and small-job progress while a large job remains active.
  - [x] Reserve the project conversion lock across delivery planning, all workers and state publication; a second conversion/plan owner is refused. Atomically publish the state cursor and preserve immutable new/prior snapshots in `Reports/ConversionHistory`.
  - [x] Fix DOS versus extended/canonical Windows path identity when finding prior hash bindings and selected retries; changed source/output hashes remain refused. Tests cover canonical-path reuse without starting a tool and malformed state refusal without overwriting the cursor.
  - [x] Verify actual LibreOffice conversion/canonical-path reuse and selected retries on four disposable synthetic RTFs; preserve original pilot outputs in a separate workstation snapshot during the path-fix recovery, without modifying captures/images/extracted sources.
- [x] Per-output timeout (default 45 s to match current workflow).
- [x] Process-tree kill on timeout.
- [x] Skip/reuse already-valid outputs; forced reconversion remains an advanced future option.
- [x] Bind reused Office/PDF outputs to saved source and output hashes across full runs and app restarts; a valid-looking but unbound or changed output is preserved and reported as an issue, not silently reused.
- [x] Validate generated Office OOXML as ZIP containers with required internal files.
- [x] Validate generated PDFs via `%PDF-` header + `%%EOF` tail sanity check.
- [x] Record `OK`, `PARTIAL`, `FAILED`, `TIMEOUT`, and `REUSED` results plus details/duration.
- [x] Preserve source stems containing extra dots when locating LibreOffice output (regression-tested with `Dr. Anka.doc`).
- [x] `conversion issues` and `conversion retry [SOURCE]` provide per-file or all-failed retries; selected retries preserve a complete project summary and revalidate unselected outputs.
- [x] Reload the last conversion issue list from project-scoped saved state after restarting the CLI process.
- [x] Single-GW background scans automatically convert eligible files, apply bounded transient retries and record failures without file-by-file prompts. Reinspect unchanged previous jobs instead of retrying each failure at every disk arrival; offline `processing resume`/`process`/`conversion retry` can explicitly retry. Full two-station resource-adaptive production policy remains separate.

## 14. Audit/report engine

Replace the current updater/audit script chain with one in-app source of truth while keeping export compatibility.

- [x] Add a clearly scoped CLI evidence audit: re-hash acquisition images, managed recovered files, and conversion source copies; validate recorded Office/PDF outputs; flag missing/changed evidence per disk and export JSON/CSV without claiming customer-delivery certification.

- [ ] Per-floppy audit state combines acquisition, image quality, extraction, recovered-file count, conversion status, and generated-file integrity.
- [ ] Preserve useful statuses such as `OK`, `PARTIAL: IMAGE READ`, `PARTIAL: CONVERSION`, `CHECK: CONVERSION FAILED`, `CHECK: NO RECOVERED FILES`.
- [ ] Summary metrics equivalent to the current final report.
- [ ] Recovered-file inventory with recovery method + SHA-256.
- [ ] Conversion result inventory and conversion-issues subset.
- [ ] Generated-file integrity inventory.
- [ ] Delivery-file manifest.
- [ ] Export CSV/text reports.
- [ ] Export `FloppyFinalReport.xlsx` equivalent from the app or a dedicated report exporter.
- [x] Generate polished XLSX reports directly rather than depending on Excel COM automation.
- [x] Primary report language is Hungarian.
- [ ] Add English report export from the same underlying report data model.
- [x] Excel summary/dashboard sheet with major KPIs and project statistics.
- [x] Include charts for useful project-wide metrics such as imaging status, bad-sector counts, recovery results, file counts, and conversion outcomes.
- [x] Detailed per-floppy worksheet/table with filtering, frozen headers, sensible column widths, status highlighting, and consistent formatting.
- [ ] Separate recovered-file, conversion, issue, and integrity tables where useful.
- [x] Reports should be presentable to a customer without requiring manual cleanup in Excel.
- [x] Audit must be re-runnable/idempotent and never alter source floppy media.
- [ ] Audit runs automatically after every material state change and at batch completion; no manual spreadsheet update step remains.

## 15. Customer package builder

Reproduce `Make-FloppyCustomerPackage_v1.ps1` in the CLI.

- [x] Choose destination outside project/source tree and enforce that guardrail.
- [x] Stage only allowed archival/customer folders in the package file list (no source-tree mutation).
- [x] Exclude internal helper/state files from customer content.
- [x] Exclude Windows `System Volume Information` / Recycle Bin folders from customer delivery and future conversion mirroring, while preserving their captured bytes in source images and forensic extraction evidence.
- [x] Include only selected customer-useful reports, plus the evidence audit and latest FluxVault workbook; exclude working notes and stale workbooks.
- [x] Generate package manifest with size, original modified timestamp (UTC), and SHA-256.
- [x] Generate manifest SHA-256 file.
- [x] Generate README explaining Images / Logs / Extracted / Converted / Recovery / Reports and known limitations.
- [x] Create timestamped ZIP.
- [x] Compress customer-package entries losslessly with Deflate level 6 instead of storing them uncompressed; preserve inventory/hash verification and internal-evidence exclusions. This does not compress working raw captures in place.
- [x] Hash final ZIP and write `.zip.sha256`.
- [x] Verify ZIP inventory/count/total bytes against staging before declaring success.
- [ ] Optional “keep staging folder” setting.
- [ ] One **Finalize project** action automatically refreshes recovery/extraction/conversion/audit state, builds the package, verifies it, and reports only unresolved exceptions.
  - [x] Add CLI `finalize --destination PATH` for existing images: run the shared processing pipeline, stop packaging when attention remains, and build/verify an archival ZIP only after a clean run. Automatic production policy remains open.
- [ ] Optional production policy automatically builds the final package when the last physical disk and all background queues are complete.

## 16. Current dataset regression targets

### First workable pilot — single Greaseweazle station

- [x] Simplify GW custody to number-only input (`004`/`4`) or `QUIT`, retain wrong-number/blank refusal and legacy READ compatibility.
- [x] Add ordinary `scan` for GW with saved per-project defaults and an installer-provided `fv` alias; preserve explicit guarded USB scans and expert overrides.
- [x] Explain built-in/saved recovery budgets as optional expert configuration rather than mandatory operator homework (`POLICIES.md`).
- [x] Reproduce/fix host exit-zero `Command Failed: No Index` reporting; preserve failed metadata and allow an operator-confirmed expired empty-first-capture restart without resetting jobs that contain raw evidence.

**Current operator-time target:** first run up to 20 customer disks (001–020), stopping earlier with `QUIT` if needed; collect real timing/recovery data before expanding to 136. Disk 009's archived image is 720 KB, so the cohort must switch formats rather than decode everything as HD. See `PILOT_20.md`.

- [x] Provide the short 20-disk runbook, opt-in 180-second/three-pass recovery policy, and archive-derived profile list; preserve the normal recovery defaults.
- [x] Support a validated per-disk profile map in guided GW scanning, persist/bind pending resume settings, record actual profiles, and switch 009 to DD/010 back to HD without technical swap-time decisions.
- [x] Preflight required downstream tools before normal GW scan custody/acquisition, with explicit acquisition-only bypass and no physical read on failure.
- [x] Test the 20-disk mixed-format cap/tail and cross-process 008/009/010 switching, changed-map restart refusal, older journals, invalid maps, and processing-preflight refusal without hardware.
- [x] Complete the live 001–020 cohort and retain telemetry: 20 committed disks, 17 acquired/three partial, 26 saved raw captures, automatic DD switch at 009, 20 extracted disks and 172 successful conversion jobs/zero conversion failures. Initial 004 failure remains recorded; resumed scanning completed the cohort.
  - [x] Exercise bounded damaged-disk recovery on 005/012/017: one unresolved sector each, no byte conflicts; native extraction recovered 3/11/15 complete files respectively. 017 skipped one entry, so recovered intact files do not prove filesystem completeness.
- [x] Compare all 20 acquired disks against original script/DMDE archive source payloads, distinguishing renamed/identical/changed/missing files, source-image differences and deleted/carved/unknown scope: 181/188 identical, one changed, six missing; preserved older managed generations verify correctly. This measures baseline differences, not full recovery parity.
  - [x] Add guarded offline `benchmark compare --baseline ZIP` with immutable JSON/CSV snapshots, streamed hashes, one-to-one disk matching, unscanned-disk exclusion and explicit comparison-only `--include-deleted`. Confirmed deleted payloads are excluded by default; uncertain/carved references stay visible/in scope.
  - [x] Compare saved 053–064: 81/88 identical; four changed/three missing rows all on 058 with live/deleted version ambiguity and different image bytes. 059's two live documents and 062's eight payloads match exactly; ten confirmed deleted 059 payloads are out of default scope.
  - [x] Provide a fresh-project/resume test launcher and an offline collect-only mode that saves a compact operator summary plus benchmark/status/queue/comparison results; validate Windows PowerShell without hardware.
  - [ ] Resolve/classify 009 carving candidates, 017 skipped live file, and 020/058 historical byte differences from verified saved evidence and controlled fresh captures; do not fabricate parity or silently classify ambiguous collision-renamed versions as deleted.
- [x] Use pilot measurements to ship visible cues, balanced/configurable conversion workers and lossless working-capture storage. Baseline: mean 108.16 s/disk, waits 13.15 min, downstream 7.89 min; raw SCP 985.13 MiB becomes 232.25 MiB on the offline packed copy. The previous 5.52-hour feed projection is not a validated full-chain result or measurement of this build.
- [x] Correct benchmark classification of downstream exit 3: count `downstream_attention` separately from `downstream_errors`; recomputing historical events now distinguishes successful processing with unresolved disks from tool failure. Retain the genuine initial 004 acquisition failure and interrupted-session history.

- [x] Provide a controlled pilot runbook with isolated projects, small live smoke-test gates, the full numbered collection, restart instructions and a local data-review recipe (`PILOT_136.md`).
- [x] Add `--last-disk 136` to guided GW scans so the same command stops at the same collection endpoint after restart; keep `--count` as a session cap and preserve older journals.
- [x] Persist synced per-invocation benchmark events with build/configuration fingerprint, confirmations, recovery/verification timing, read counts, missing/conflicting LBAs, hashes, failures and downstream results; retain interrupted-session evidence without overwriting previous runs.
- [x] Export benchmark JSON/per-disk CSV automatically and through offline `benchmark report`; deduplicate resumed numbering, retain partial statuses, flag truncated tails, and reject malformed committed records. Keep internal telemetry outside customer packages.
- [x] Simulate a 136-disk single-station scan with partial disks, failure at disk 061, persisted resume, unique yield/error accounting and a restart-safe stop at 136.
- [ ] Pass small clean/damaged live smoke tests with telemetry before starting the full pilot; inspect fixed-profile behavior on known DD/nonstandard media.
  - [x] Customer 007 live guided smoke test: 2,880/2,880 sectors, one capture, 18 extracted files with all payload hashes matching the archived originals, 18 successful document conversions, clean evidence audit/workbook and benchmark exports. Whole-image hashes differ at five LBAs; do not claim byte-identical media or infer the cause.
  - [x] Customer 009 live mapped-DD smoke test: 1,440/1,440 sectors, one capture, exact 737,280-byte image, one reachable FAT file, clean evidence audit/workbook and benchmark exports; actual profile switch used with the short policy.
  - [ ] Compare live 009 against the archive's signature-carved outputs (including a large legacy Word candidate and JPEG), distinguishing deleted/orphaned content, recovery-tool reports and actual reachable files. A clean FAT extraction is not equivalence to legacy carving, and whole-image/payload hashes currently differ.
- [ ] Run the first 136-disk physical pilot and collect benchmark/audit/recovery artifacts, preserving the original script archive for comparison.
- [ ] Compare pilot source/recovered-file hashes and yield against the script/DMDE baseline, then prioritize changes using measured failure/throughput data.

Use the supplied `FloppyFinalReport.xlsx` and existing archive as regression truth while porting functionality.

- [ ] Import/represent all 136 floppy records.
- [ ] Current reference summary: 136 images present; 94 imaging OK; 42 imaging not OK; 86 floppies fully OK; 50 need attention/are partial.
- [ ] Reproduce 1,667 recovered source-file records and the current conversion/audit counts when pointed at the same archive contents.
- [ ] Correctly represent severe cases rather than assuming every image is 1.44 MB; current data includes manually recovered/high-error cases and at least one 417,792-byte image.
- [ ] Regression-test examples with 1 bad sector, tens of bad sectors, hundreds of bad sectors, conversion-only failures, no-recovered-file cases, manual recovery, and signature recovery.
- [ ] Measure automated recovery yield against the existing manual DMDE/script results; FluxVault must match or exceed recovered verified files wherever the same evidence is available.
- [ ] Track operator interventions required for all 136 disks and drive the normal technical-decision count toward zero.
- [ ] Benchmark a simulated/fixture-based two-drive run before using customer media, including queue scheduling and crash-resume behavior.

## 17. Testing

- [x] Unit tests for floppy-number parsing and zero-padding.
- [x] Unit tests for legacy archiver-log parsing.
- [x] Unit tests for DMDE multi-pass map replay (later successful `C` replaces earlier `E`).
- [x] Unit tests for path cleanup / delivery naming / collision handling.
- [x] Unit tests for Greaseweazle command construction, especially raw-flux safety flags.
- [ ] Unit tests for project persistence and migrations.
- [x] Unit tests for SHA/integrity helpers.
- [x] Fixture-based tests using scrubbed/sample logs and tiny synthetic images; never require a customer floppy for automated tests.
- [x] Native FAT12 fixtures cover fragmented chains, directory gaps, FAT-copy fallback/conflicts, FAT entries crossing sector boundaries, loops/cross-links, unsafe paths and 701-bad-sector recovery of an independently intact file.
- [x] Native service/CLI tests cover hash/map refusal, immutable source/manual preservation, inventory/report tampering, verified reuse and partial files flowing through batch extraction, manifests, delivery mirroring and audit with no hardware.
- [x] Preserve valid DOS installer underscores and escape-prefix uniqueness; select managed generations numerically so legacy native output cannot hide a later numbered extraction.
- [x] VFAT name fixtures cover Unicode/surrogate pairs, exact 13-unit boundaries, padding/type/checksum errors, directory gaps/deleted entries, fragmented-directory slots, unsafe names, alias/Unicode collisions and ASCII short-name case flags.
- [x] Native generation upgrade test preserves first-generation files/reports/schema compatibility; OS metadata remains forensic but is omitted from new delivery mirrors and verified packages.
- [x] Validate native recovery on the saved WinWord 1 Greaseweazle image: 22 intact files, unchanged image hash, zero new physical reads, repeat reuse and downstream partial audit/workbook.
- [x] Integration test for 7-Zip adapter.
- [x] Integration test for LibreOffice adapter when installed.
- [x] Greaseweazle hardware tests marked/isolated so normal `cargo test` works without hardware.
- [ ] End-to-end automated fixture test: acquisition artifact -> triage -> extraction/recovery -> conversion -> audit -> verified package with no technical operator choices.
- [ ] Scheduler tests prove USB and Greaseweazle jobs can run concurrently without disk-number or artifact cross-contamination.
- [ ] Policy tests cover automatic escalation, bounded retries, no-improvement stopping, severe-damage carving, and unrecoverable outcomes.
  - [x] Mock tests cover clean fast-pass stop, targeted escalation, no-improvement stop, recovered-sector provenance, control-byte conflict refusal, absent-board refusal, policy-limit validation, output tamper refusal, and offline decode resume after expiry with no board.
- [ ] Long-run soak test models 136 disks, application restart, worker failure, and resumability.
- [ ] Clear legacy Clippy warnings and enforce strict all-target linting; formatting, all-target checking and automated tests currently pass, but strict `-D warnings` linting does not yet pass.
  - [ ] Investigate one intermittent Windows concurrent test-harness fast-fail (`0xc0000409`) observed during storage development; isolated storage tests and subsequent complete reruns passed. Do not treat the unreproduced event as a diagnosed/fixed defect; keep it in soak-test acceptance.

## 18. CLI / automation interface

All CLI commands must call the same guarded Rust workflow services so safety, provenance, validation, and output formats cannot drift.

- [ ] Install a `fluxvault` executable that can be added to `PATH` and run from PowerShell, CMD, or another automation process.
  - [x] Build and smoke-test a release executable in PowerShell and CMD; provide a dry-run-capable installer with opt-in user `PATH` changes. Actual installation is left to the operator.
- [x] Discover a project by walking upward from the current directory, like Git, with an explicit `--project <path>` override for the first CLI status command.
- [x] `fluxvault init [path]` creates a project in the current or supplied directory; `fluxvault status` summarizes its health and next required actions.
- [x] Project commands: `project show`, `disk list`, `disk show`, `disk select`, and `disk next`.
  - [x] Read-only `disk list` and `disk show N` with human/JSON output and no physical drive access.
  - [x] `disk show N --details` exposes saved attempt hashes, bad LBAs, retry counts, and evidence paths; JSON includes these fields automatically.
- [x] Read-only drive commands: `drive list` and `drive probe --drive A:`. Probe only accepts an enumerated removable drive, performs no write, and reports whether software guards pass; the current USB adapter still needs independent hardware write-protection validation before customer use.
- [ ] Acquisition commands: `acquire --drive A: --disk N --retries N` plus a production `scan` workflow where the only interaction is media-change confirmation.
  - [x] Add `greaseweazle recover N --gw-drive B --source-write-protected [--policy FILE] [--acquisition-only]` with default downstream processing, JSON output, durable resume, and live protected-media validation.
  - [x] Add `greaseweazle scan` with numbered custody prompts, durable completion/cursor recovery, partial continuation, automatic downstream tail, JSON summaries, and cross-project CLI drive reservation; mock unit/executable tests cover failure/restart/termination.
  - [x] Add gated CLI `acquire` using the read-only USB backend, positive write-protection and floppy-geometry checks, and a required operator hardware-protection assertion. Live read-only test on customer 007 completed with one unresolved sector; the other 2,879 sectors exactly matched its earlier clean image. Independent physical write-protection validation remains open above.
  - [x] Add a guarded, guided single-drive `scan` loop that requires an explicit READ confirmation for each disk, advances project numbering only after a completed image, and reports the derived recovery queue. Tested with synthetic acquisition, not live hardware.
  - [ ] Turn guided scanning into the production zero-touch pipeline: automatic post-scan extraction/recovery decisions, crash-safe resume, reliable media-change detection, and independent hardware validation.
- [x] Extraction commands for one disk or all eligible disks, preserving manual-recovery detection and immutable recovery backups.
  - [x] `extract all` uses the shared batch service, recovery backups, manual-recovery detection, and manifest generation without requiring LibreOffice.
  - [x] `extract disk N` uses the same eligibility rules, preserves operator recovery, makes/reuses the immutable pass-1 backup when needed, and refreshes the file manifest.
  - [x] `recovery extract N` invokes native FAT12 recovery on saved evidence without 7-Zip, LibreOffice or hardware; refreshes the manifest, supports JSON and returns attention code 3 rather than certifying complete recovery.
- [x] Recovery commands for queue/status, attempt comparison, composite creation, and DMDE result import.
  - [x] `recovery plan`, `recovery compare N`, and idempotent `recovery backup N` operate on saved evidence without physical-drive access.
  - [x] `recovery queue`, `recovery composite N`, `recovery fat N`, and guarded `recovery import N --source DIR --dmde-log FILE` use the saved-image and evidence-import services.
- [x] Conversion, audit/report, and validated archival-package commands are available from the CLI; full customer-delivery certification remains open.
  - [x] `report export` uses the Hungarian XLSX exporter; `tools check` uses version checks and the audited external-command runner.
  - [x] `conversion plan`, `conversion run`, and `files manifest` use delivery planning, bounded conversion, and inventory services.
  - [x] Make selected conversion-issue retry resumable after CLI process restart without weakening source provenance checks. CLI `conversion run`/`process` save project-scoped state; `conversion retry [SOURCE]` reloads it and validates fresh source hashes, delivery paths, and existing outputs.
- [ ] Human-readable output by default plus stable `--json` output for scripts; progress goes to stderr so JSON/stdout remains machine-readable.
- [ ] Stable documented exit codes for success, partial recovery, operator action required, invalid project/input, missing tool, and fatal failure.
  - [x] Initial CLI contract: 0 complete, 3 attention/partial, 2 invalid input/operation error; `audit` and `process` return 3 when recovery or conversion attention remains.
- [ ] Non-interactive/background operations require explicit policy flags; physical-media operations retain read-only safety while confirmations are limited to unavoidable custody/media changes.
- [x] Every CLI external-tool invocation uses argument arrays and the shared command/audit log; never expose Greaseweazle write/erase commands.
- [ ] Shell completion generation for PowerShell initially, with Bash/Zsh completion when the application becomes cross-platform.
- [ ] CLI integration tests cover project discovery, JSON schemas, exit codes, resumability, and safe failure without physical hardware.
  - [x] Exercise the built executable against a disposable nested project: project discovery, JSON output/errors, exact exit codes, and a guided-scan quit path that never enumerates or reads a drive.
  - [x] Cross-process LibreOffice integration test proves hash-bound reuse, tampered-output refusal, persisted issue loading, and retry after restart using only a disposable RTF.
  - [x] Cross-process GW recovery test seeds saved raw evidence, publishes a compatible image, then reuses the completed result with the mock board absent and no new read commands.
  - [ ] Add interrupted-acquisition resume integration scenarios without requiring physical media.
- [ ] `fluxvault production start` runs the shared two-drive scheduler and prints concise USB/GW swap instructions while all technical decisions remain automatic.
- [x] `fluxvault audit` and `fluxvault package build --destination PATH` use the evidence-audit and verified-package services without application-wide project state.
- [x] `fluxvault process` runs the existing-image extraction -> Office conversion -> evidence audit -> workbook pipeline with no physical drive access; progress goes to stderr and `--json` output to stdout.

## 19. Milestones

- [x] **M0 — CLI foundation:** module layout, settings, project create/open, guarded workflow services, and folder-first commands. The earlier desktop shell was retired.
- [x] **M1 — WORKING USB ARCHIVER:** safely image a real floppy, retry/fallback, bad-sector map, SHA-256, persistent project record.
- [ ] **M2 — SCRIPT REPLACEMENT CORE:** import legacy archives/logs, auto extraction, recovery queue, manifests.
- [ ] **M3 — AUTOMATED RECOVERY CORE:** multiple USB attempts, policy-driven compare/composite, FAT12 reconstruction, carving, provenance, and legacy DMDE import compatibility.
- [ ] **M4 — GREASEWEAZLE READY WITHOUT HARDWARE:** tool detection, mocked backend, safe command construction, raw/derived artifact model.
- [ ] **M5 — GREASEWEAZLE LIVE:** raw-flux capture + decode/redecode + detailed flux diagnostics after hardware arrives.
- [ ] **M6 — COMPLETE SUITE:** Office conversion, integrity, audit workbook/report exports, customer ZIP packaging.
- [ ] **M7 — HARDENING:** recovery regression tests, crash/cancel behavior, settings polish, release build.
- [ ] **M8 — ZERO-TOUCH PRODUCTION:** concurrent USB + Greaseweazle scheduler, automatic escalation, one-button downstream pipeline, 136-disk afternoon benchmark, and near-minimum operator touches.

## 20. First implementation session after repository is created

- [x] Read the new GitHub repository exactly as pushed.
- [x] Earlier GUI/DPI groundwork completed historically; retired when FluxVault became CLI-only.
- [x] Add the initial dependencies; later remove GUI-only dependencies and the DPI manifest.
- [x] Split the hello-world project into the initial module skeleton.
- [x] Create the original GUI shell; later retire it in favor of the CLI.
- [x] Add project create/open and settings/tool-health structures.
- [x] Push the first checkpoint even if some planned pieces are still stubbed.
- [x] Then start the Windows read-only floppy backend immediately; do not spend three days making pretty cards before we can read a disk. :D
