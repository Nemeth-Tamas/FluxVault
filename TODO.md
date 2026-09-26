# FluxVault — TODO

> Floppy archival, forensic imaging, recovery, conversion, audit, and customer-delivery suite.
>
> **Primary rule:** Source floppy media is read-only. FluxVault may write images, logs, extracted files, reports, and packages to the workstation, but it must never intentionally write to a customer floppy.
>
> **Product target:** FluxVault is an automated archival appliance, not a collection of expert-only recovery tools. Except for physically inserting, removing, or moving a floppy between drives, the normal operator workflow should require no recovery decisions, no manual DMDE work, no hand-edited spreadsheets, no manual extraction, and no manual report/package assembly. The intended production loop is: **insert floppy -> press/confirm once -> wait for the swap cue -> repeat**.
>
> **Throughput target:** With one USB floppy drive and one Greaseweazle-connected drive operating concurrently on different disks, a 136-disk mixed-condition job—including automatic verification, escalation, extraction, conversion, audit, and ordinary recovery passes—should be achievable within one operator afternoon (target: no more than roughly 6 hours of attended wall-clock time, excluding genuinely pathological media that must continue unattended or be reported as unrecoverable).

## 0. Development contract / project rules

- [x] Rust stable, Windows-first application.
- [x] GUI: `eframe` / `egui` unless we discover a concrete blocker.
- [x] Use the same Windows DPI-manifest pattern already proven in BareEye / QuadBench / EagleCast: `embed_manifest` + `DpiAwareness::System` in `build.rs`.
- [ ] Keep the GUI responsive: floppy reads, hashing, extraction, conversion, packaging, and Greaseweazle processes run on worker threads/processes and report progress/events back to the UI.
- [x] User edits files locally; assistant does not hand-wave patches.
- [x] Before every code patch, assistant reads the current file from GitHub and supplies exact FIND and REPLACE blocks with indentation copied from the repository.
- [x] Intermediate states are pushed even when they do not compile, so the repository is always the source of truth.
- [x] Git workflow always uses `git add .`.
- [x] Do not use selective `git add <file>` instructions.
- [x] Prefer small modules with explicit responsibilities over a giant `main.rs`.
- [x] Errors shown to the operator must preserve the underlying technical detail in logs.

### Automation-first operator contract

- [ ] The default workflow must be a guided production queue, not a page-by-page collection of manual actions.
- [ ] The operator's only routine responsibilities are placing/removing disks, moving a disk from USB to Greaseweazle when prompted, and optionally entering a physical label or note.
- [ ] FluxVault automatically chooses retries, read direction, composite inputs, extraction strategy, recovery escalation, conversion, audit, and packaging policy from recorded evidence.
- [ ] Expert controls remain available under **Advanced**, but normal jobs must not require understanding sectors, FAT, DMDE, flux, profiles, hashes, or conversion filters.
- [ ] Every automatic decision records its evidence, confidence, limits, and provenance so automation never hides guessing or fabricates recovered data.
- [ ] A disk may finish as **verified**, **partially recovered**, or **unrecoverable within policy**; the program must not block the entire batch waiting for manual repair.
- [ ] All stages are resumable after application restart or workstation failure without repeating completed evidence-preserving work.
- [ ] No modal question should ask the operator to make a technical choice the program can derive safely.

## 1. Safety invariants — must exist before real media testing

- [x] Create a central `MediaSafetyPolicy` / equivalent that marks all physical-floppy operations as READ ONLY.
- [x] Windows USB-floppy backend opens `\\.\A:` (or selected drive) with read access only; never request write access.
- [x] No code path may call a filesystem write operation against the floppy drive letter.
- [x] Greaseweazle integration exposes acquisition/info/convert operations only.
- [x] Never expose or invoke `gw write`, erase, clean, or another destructive Greaseweazle operation.
- [x] Show a persistent **SOURCE MEDIA: READ ONLY** indicator whenever a physical drive is selected.
- [x] Recommend the physical write-protect tab for customer disks when available.
- [x] Query Windows disk writability without attempting a write; refuse a full USB image if protection is not positively reported.
- [ ] Validate the current USB floppy drive's write-protect reporting with a **known-good disposable** floppy only. It reported `writable` with the physical tab open, and Windows-created filesystem metadata appeared between archived and fresh customer-disk images. On damaged disposable 001, a deliberate one-byte marker write to a previously readable sector failed with device I/O error 1117; the immediate read failed, but after eject/reinsert the sector again matched its archived SHA-256 exactly. This confirms no persistent change to that sector, **not** that the adapter enforces write protection. Do not test writes on customer media; treat this drive as unsafe for further customer insertions until the discrepancy is resolved or use a verified hardware write blocker/GW setup.
- [x] Keep a command/audit log for every external tool invocation.
- [x] Never silently overwrite a previous acquisition/recovery attempt.

## 2. Project/session data model

- [x] Define a FluxVault project root while remaining compatible with the current archive layout during migration.
- [x] Recognize/use the existing directories where present: `Images`, `Logs`, `Extracted`, `Converted`, `Recovery`, `Reports`.
- [x] Add `Flux` (or equivalent) for raw Greaseweazle captures.
- [ ] Add a small FluxVault project metadata file (`project.json` or similar) containing project name, created time, operator settings, next floppy number, and tool paths/versions.
- [ ] Model each floppy as a stable record with zero-padded number (`001`, `002`, ...), label/notes, acquisition attempts, current preferred image, extraction state, recovery state, conversion state, and audit state.
- [x] Model acquisition attempts as immutable records: source backend, timestamp, geometry/format, output artifacts, hashes, bad-sector map, status, and log path.
- [ ] Allow one attempt to be marked **preferred/current** without deleting older attempts.
- [ ] Preserve enough provenance to answer: “Which read/pass produced this sector/file?”
- [ ] Import an existing script-created archive as a project without forcing re-imaging.

## 3. GUI shell

- [x] Main window with left navigation and central work area.
- [x] Suggested pages: **Project**, **Acquire**, **Recovery**, **Files**, **Conversions**, **Audit**, **Package**, **Settings / Tools**.
- [x] Project header: project path, disk count, next number, active source device, current job.
- [x] Bottom status area: worker status, progress, current operation, last error/warning.
- [ ] Non-blocking media-change prompt for “Insert floppy #NNN in USB”, “Move floppy #NNN to Greaseweazle”, or “Archive floppy #NNN and insert #NNN+1”.
- [x] Operator log panel with timestamps and copy button.
- [ ] Persistent per-job cancel button where cancellation is safe.
- [ ] Do not fake capabilities: controls for unavailable hardware/tools are disabled with a useful reason.
- [ ] Add a production-line dashboard showing both drives, current disk in each drive, queued escalation work, throughput, estimated remaining time, and the next physical action in plain language.
- [ ] Provide a kiosk/large-button mode suitable for repetitive scanning where the primary controls are **Start batch**, **Disk inserted**, **Disk removed**, **Pause**, and **Finish batch**.

## 4. External-tool discovery

- [x] Tool manager detects/configures:
  - [x] 7-Zip (`7z.exe` / `7zz.exe` / `7za.exe`).
  - [x] LibreOffice (`soffice.com` preferred, `soffice.exe` fallback).
  - [x] Greaseweazle host tools (`gw.exe`) when installed later.
- [x] Store operator-selected paths in settings.
- [x] Show detected version and health check for each tool.
- [x] Provide a “Test tools” action.
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
- [x] Route these cases to Recovery instead of pretending success:
  - [x] non-clean image / unreadable sectors;
  - [x] missing/unrecognized acquisition log;
  - [x] unfinished acquisition log is preserved separately as **IN PROGRESS**;
  - [x] FAT listing failure;
  - [x] extraction failure;
  - [x] apparently readable image with zero recovered files when operator review is warranted.
- [ ] Automatically run extraction immediately after an eligible acquisition or newly derived preferred image; no separate Files-page action in production mode.
  - [x] In the current GUI session, queue image-only extraction after each clean USB acquisition without blocking the next physical read.
  - [x] Add a one-button offline project pass that batches eligible extraction, conversion, evidence audit, and a Hungarian workbook without touching physical media.
- [ ] Automatically re-run extraction and file inventory whenever a better composite, decoded flux image, or reconstructed filesystem becomes preferred.
- [ ] Replace “operator review required” as the normal next step with a bounded automatic recovery plan; operator review is the final exception state only.
- [ ] Treat existing manual recovery folders/DMDE imports as legacy compatibility inputs, not as the intended future recovery workflow.

## 8. Automated recovery engine — pre-Greaseweazle

- [x] Recovery queue ordered by severity/attention state.
- [ ] Show current image, bad-sector count/list, source log, extraction result, prior attempts, automated decisions, and optional operator notes in one screen.
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
- [ ] Use both FAT copies, boot-sector/BPB evidence, root-directory entries, cluster chains, file sizes, and cross-attempt sector provenance to reconstruct damaged filesystems without arbitrary byte guessing.
- [ ] Add automatic deleted/orphaned cluster-chain recovery where FAT12 evidence supports it, clearly labeling confidence and recovery method.
- [ ] Add signature-based file carving as an automatic fallback for unreconstructable filesystems, preserving raw offsets and labeling filenames/paths as reconstructed.
- [ ] Detect common document/archive/image signatures and validate carved outputs before including them in customer delivery.
- [ ] Try multiple evidence-ranked interpretations automatically and retain all non-destructive candidates; never require the operator to choose a sector manually.
- [ ] For disks with hundreds of bad sectors, recover every independently verifiable file/fragment possible, then produce a precise unrecoverable-range report instead of failing the entire disk.
- [ ] Automatically prefer a more complete recovery while retaining prior results and explaining why the preferred result changed.
- [ ] Replace reliance on interactive DMDE with native Rust recovery or another fully automatable, auditable read-only engine. DMDE may remain an optional compatibility/fallback adapter only if it can be automated legally and safely.

## 9. Greaseweazle integration — hardware-independent groundwork

Greaseweazle host tools are intentionally wrapped rather than reimplemented initially. Current upstream supports Windows `gw.exe`, raw-flux formats including SCP/KryoFlux, and a separate `gw convert` path, so we can build/test command generation and output parsing before the board arrives.

- [x] Create `GreaseweazleBackend` abstraction with a mock/no-hardware mode.
- [ ] Detect `gw.exe`, run info/version command, and show device status.
- [x] Build commands as argument arrays, never shell-concatenated strings.
- [x] Unit-test command generation without hardware.
- [ ] Parse `gw` stderr/stdout incrementally into GUI progress/events.
- [ ] Store full command, version, start/end time, exit status, and captured output for every run.
- [ ] Add “hardware not connected” UI state rather than error-spamming.

## 10. Greaseweazle raw-flux acquisition — after board arrives

- [ ] Detect board + connected drive and display device/firmware info.
- [ ] **Preservation capture defaults to true raw flux**, e.g. SCP/KryoFlux, not regenerated “perfect” flux.
- [x] Important guardrail: if `gw read --format=...` is used for a raw-flux file, pair it with `--raw`; otherwise Greaseweazle may regenerate flux and fill undecodable sectors rather than preserving the physical capture.
- [ ] Default recovery workflow: automatically capture raw flux once when USB triage escalates a disk, then perform as much decoding/re-decoding as possible from that preserved capture instead of repeatedly stressing fragile media.
- [ ] Allow configurable revolutions for raw capture where the selected image type supports it.
- [ ] Preserve every raw acquisition as an immutable attempt with SHA-256.
- [ ] Derive sector images from raw captures using `gw convert --format=<profile>`; derived images are separate artifacts, never replacements for raw flux.
- [x] Profiles initially required for this collection:
  - [x] IBM PC 1.44 MB / HD.
  - [x] IBM PC 720 KB / DD.
- [ ] Later expose other Greaseweazle disk definitions without hardcoding the whole universe into FluxVault.
- [ ] Track/head selection and step settings available under **Advanced**, not in the basic happy path.
- [ ] Apply bounded automatic physical-read policies based on media condition, elapsed time, revolutions, and prior improvement; stop automatically rather than endlessly hammering fragile media.
- [ ] Automatically infer the first decode profile from USB geometry/image size and flux evidence, then try evidence-ranked alternative profiles without operator selection.
- [ ] After flux capture, automatically decode, compare against USB attempts, build the best composite, retry extraction/recovery, and update audit state.
- [ ] Tell the operator exactly when to move a USB-problem disk into the Greaseweazle drive and when it can be removed; no flux expertise should be required.

## 11. Autonomous two-drive production workflow

The target setup has two different drives working simultaneously on different floppies: the USB drive performs fast first-pass acquisition while the Greaseweazle drive processes disks automatically escalated from the USB queue. A single disk is never placed in both drives simultaneously; the scheduler tracks custody and tells the operator where each numbered disk goes next.

- [ ] Create a central job scheduler shared by GUI and CLI, with independent USB, Greaseweazle, CPU extraction/recovery, conversion, audit, and packaging worker queues.
- [ ] Run the USB and Greaseweazle physical drives concurrently on different disks without blocking hashing, extraction, conversion, or reporting workers.
- [ ] Automatically triage every USB result into **USB complete**, **USB re-read**, **move to Greaseweazle**, or **unrecoverable within USB policy**.
- [ ] Automatically prioritize the Greaseweazle queue by expected recovery value, severity, age, and whether the operator currently has the disk available.
- [ ] Maintain unambiguous disk identity/custody so results from two drives can never be attached to the wrong floppy number.
- [ ] Require a simple physical confirmation when moving a disk between stations, then verify geometry/fingerprint consistency before accepting the new attempt.
- [ ] Keep both drives busy whenever eligible work exists; CPU-heavy extraction/conversion must not stall physical acquisition.
- [ ] Allow the operator to continue feeding good disks into USB while Greaseweazle works on an earlier bad disk.
- [ ] Use audible and large visual cues differentiated by station: **USB swap**, **move to Greaseweazle**, **Greaseweazle swap**, and **attention only if automation is exhausted**.
- [ ] Support pause/resume and clean shutdown while preserving every queue item and in-progress artifact safely.
- [ ] Estimate throughput and remaining batch time from observed read/retry/conversion durations.
- [ ] Add a production acceptance benchmark for the 136-disk reference job: complete ordinary dual-drive acquisition/recovery and downstream processing within a target six-hour operator session.
- [ ] Record operator touches per disk and target the theoretical minimum: initial insertion/removal plus one Greaseweazle transfer only for escalated disks.
- [ ] Provide an unattended tail mode so flux re-decodes, extraction, conversion, audit, and packaging can continue after the operator finishes feeding physical disks.

## 12. “Mini electron microscope the shit out of it” flux recovery view

- [ ] Track/head map for raw-flux capture quality.
- [ ] Per-track decoded sector summary: present, valid CRC, bad CRC, missing, duplicates/unusual IDs where available.
- [ ] Compare multiple revolutions/passes visually.
- [ ] Show weak/problematic regions and which decode attempt recovered each sector.
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
- [x] Support current source extensions/plans:
  - [x] Word-family -> DOCX + PDF: `.doc`, `.rtf`, `.wps`, `.wri`, `.wpd`, `.sdw`.
  - [x] Spreadsheet-family -> XLSX + PDF: `.xls`, `.xlw`, `.xlt`, `.wk1`, `.wk3`, `.wk4`, `.wks`, `.123`, `.wb1`, `.wb2`, `.wq1`, `.wq2`, `.sdc`.
  - [x] Presentation-family -> PPTX + PDF: `.ppt`, `.pps`, `.pot`, `.sdd`.
- [x] Bounded parallel Office conversion (four workers by default, CLI-configurable from 1 to 16), with isolated LibreOffice profiles, ordered reports, and serialized command audit records.
- [x] Per-output timeout (default 45 s to match current workflow).
- [x] Process-tree kill on timeout.
- [x] Skip/reuse already-valid outputs; forced reconversion remains an advanced future option.
- [ ] Bind reused Office/PDF outputs to saved source and output hashes across full runs and app restarts; structural ZIP/PDF validity alone does not prove an output belongs to the current recovered source.
- [x] Validate generated Office OOXML as ZIP containers with required internal files.
- [x] Validate generated PDFs via `%PDF-` header + `%%EOF` tail sanity check.
- [x] Record `OK`, `PARTIAL`, `FAILED`, `TIMEOUT`, and `REUSED` results plus details/duration.
- [x] Preserve source stems containing extra dots when locating LibreOffice output (regression-tested with `Dr. Anka.doc`).
- [x] Conversion issues list in the GUI session with per-file selection, retry selected, and retry failed actions; selected retries preserve a complete project summary and revalidate unselected outputs.
- [ ] Reload the last conversion issue list from persisted reports after reopening a project, so a GUI restart does not require another full conversion run to show prior exceptions.
- [ ] Production mode automatically converts all eligible files, retries transient failures within policy, and records permanent failures without asking the operator file-by-file. The current offline/GUI conversion runner now performs one bounded retry for confirmed-clean timeouts, nonzero LibreOffice exits, and missing/invalid newly generated output; wiring this into the continuous production scheduler remains open.

## 14. Audit/report engine

Replace the current updater/audit script chain with one in-app source of truth while keeping export compatibility.

- [x] Add a clearly scoped evidence audit in GUI and CLI: re-hash acquisition images, managed recovered files, and conversion source copies; validate recorded Office/PDF outputs; flag missing/changed evidence per disk and export JSON/CSV without claiming customer-delivery certification.

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

Reproduce `Make-FloppyCustomerPackage_v1.ps1` in the GUI.

- [x] Choose destination outside project/source tree and enforce that guardrail.
- [x] Stage only allowed archival/customer folders in the package file list (no source-tree mutation).
- [x] Exclude internal helper/state files from customer content.
- [x] Exclude Windows `System Volume Information` / Recycle Bin folders from customer delivery and future conversion mirroring, while preserving their captured bytes in source images and forensic extraction evidence.
- [x] Include only selected customer-useful reports, plus the evidence audit and latest FluxVault workbook; exclude working notes and stale workbooks.
- [x] Generate package manifest with size, original modified timestamp (UTC), and SHA-256.
- [x] Generate manifest SHA-256 file.
- [x] Generate README explaining Images / Logs / Extracted / Converted / Recovery / Reports and known limitations.
- [x] Create timestamped ZIP.
- [x] Hash final ZIP and write `.zip.sha256`.
- [x] Verify ZIP inventory/count/total bytes against staging before declaring success.
- [ ] Optional “keep staging folder” setting.
- [ ] One **Finalize project** action automatically refreshes recovery/extraction/conversion/audit state, builds the package, verifies it, and reports only unresolved exceptions.
- [ ] Optional production policy automatically builds the final package when the last physical disk and all background queues are complete.

## 16. Current dataset regression targets

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
- [x] Integration test for 7-Zip adapter.
- [x] Integration test for LibreOffice adapter when installed.
- [x] Greaseweazle hardware tests marked/isolated so normal `cargo test` works without hardware.
- [ ] End-to-end automated fixture test: acquisition artifact -> triage -> extraction/recovery -> conversion -> audit -> verified package with no technical operator choices.
- [ ] Scheduler tests prove USB and Greaseweazle jobs can run concurrently without disk-number or artifact cross-contamination.
- [ ] Policy tests cover automatic escalation, bounded retries, no-improvement stopping, severe-damage carving, and unrecoverable outcomes.
- [ ] Long-run soak test models 136 disks, application restart, worker failure, and resumability.

## 18. CLI / automation interface

The GUI and CLI must call the same Rust workflow/services so safety, provenance, validation, and output formats cannot drift.

- [ ] Install a `fluxvault` executable that can be added to `PATH` and run from PowerShell, CMD, or another automation process.
  - [x] Build and smoke-test a release executable in PowerShell and CMD; provide a dry-run-capable installer with opt-in user `PATH` changes. Actual installation is left to the operator.
- [x] Discover a project by walking upward from the current directory, like Git, with an explicit `--project <path>` override for the first CLI status command.
- [x] `fluxvault init [path]` creates a project in the current or supplied directory; `fluxvault status` summarizes its health and next required actions.
- [x] Project commands: `project show`, `disk list`, `disk show`, `disk select`, and `disk next`.
  - [x] Read-only `disk list` and `disk show N` with human/JSON output and no physical drive access.
- [x] Read-only drive commands: `drive list` and `drive probe --drive A:`. Probe only accepts an enumerated removable drive, performs no write, and reports whether software guards pass; the current USB adapter still needs independent hardware write-protection validation before customer use.
- [ ] Acquisition commands: `acquire --drive A: --disk N --retries N` plus a production `scan` workflow where the only interaction is media-change confirmation.
  - [x] Add gated CLI `acquire` using the GUI's read-only USB backend, positive write-protection and floppy-geometry checks, and a required independently verified hardware protection assertion. Hardware execution remains untested; the current adapter is not approved for customer media.
  - [x] Add a guarded, guided single-drive `scan` loop that requires an explicit READ confirmation for each disk, advances project numbering only after a completed image, and reports the derived recovery queue. Tested with synthetic acquisition, not live hardware.
  - [ ] Turn guided scanning into the production zero-touch pipeline: automatic post-scan extraction/recovery decisions, crash-safe resume, reliable media-change detection, and independent hardware validation.
- [x] Extraction commands for one disk or all eligible disks, preserving manual-recovery detection and immutable recovery backups.
  - [x] `extract all` reuses the GUI batch service, recovery backups, manual-recovery detection, and manifest generation without requiring LibreOffice.
  - [x] `extract disk N` uses the same eligibility rules, preserves operator recovery, makes/reuses the immutable pass-1 backup when needed, and refreshes the file manifest.
- [x] Recovery commands for queue/status, attempt comparison, composite creation, and DMDE result import.
  - [x] `recovery plan`, `recovery compare N`, and idempotent `recovery backup N` operate on saved evidence without physical-drive access.
  - [x] `recovery queue`, `recovery composite N`, `recovery fat N`, and guarded `recovery import N --source DIR --dmde-log FILE` reuse the GUI's saved-image and evidence-import services.
- [ ] Conversion, audit/report, and validated customer-package commands matching the GUI workflow.
  - [x] `report export` reuses the GUI's Hungarian XLSX exporter; `tools check` reuses version checks and the audited external-command runner.
  - [x] `conversion plan`, `conversion run`, and `files manifest` reuse the GUI's delivery planning, bounded conversion, and inventory services.
  - [x] Make selected conversion-issue retry resumable after CLI process restart without weakening source provenance checks. CLI `conversion run`/`process` save project-scoped state; `conversion retry [SOURCE]` reloads it and validates fresh source hashes, delivery paths, and existing outputs.
- [ ] Human-readable output by default plus stable `--json` output for scripts; progress goes to stderr so JSON/stdout remains machine-readable.
- [ ] Stable documented exit codes for success, partial recovery, operator action required, invalid project/input, missing tool, and fatal failure.
  - [x] Initial CLI contract: 0 complete, 3 attention/partial, 2 invalid input/operation error; `audit` and `process` return 3 when recovery or conversion attention remains.
- [ ] Non-interactive/background operations require explicit policy flags; physical-media operations retain read-only safety while confirmations are limited to unavoidable custody/media changes.
- [x] Every CLI external-tool invocation uses argument arrays and the same command/audit log as the GUI; never expose Greaseweazle write/erase commands.
- [ ] Shell completion generation for PowerShell initially, with Bash/Zsh completion when the application becomes cross-platform.
- [ ] CLI integration tests cover project discovery, JSON schemas, exit codes, resumability, and safe failure without physical hardware.
  - [x] Exercise the built executable against a disposable nested project: project discovery, JSON output/errors, exact exit codes, and a guided-scan quit path that never enumerates or reads a drive.
  - [ ] Add full cross-process conversion-retry and interrupted-acquisition resume integration scenarios without requiring physical media.
- [ ] `fluxvault production start` runs the shared two-drive scheduler and prints concise USB/GW swap instructions while all technical decisions remain automatic.
- [x] `fluxvault audit` and `fluxvault package build --destination PATH` reuse the same evidence-audit and verified-package services as the GUI, without changing the GUI's remembered project.
- [x] `fluxvault process` runs the existing-image extraction -> Office conversion -> evidence audit -> workbook pipeline with no physical drive access; progress goes to stderr and `--json` output to stdout.

## 19. Milestones

- [x] **M0 — Skeleton:** eframe window, DPI fix, module layout, settings, project open/create, worker/event plumbing.
- [x] **M1 — WORKING USB ARCHIVER:** safely image a real floppy, retry/fallback, bad-sector map, SHA-256, persistent project record.
- [ ] **M2 — SCRIPT REPLACEMENT CORE:** import legacy archives/logs, auto extraction, recovery queue, manifests.
- [ ] **M3 — AUTOMATED RECOVERY CORE:** multiple USB attempts, policy-driven compare/composite, FAT12 reconstruction, carving, provenance, and legacy DMDE import compatibility.
- [ ] **M4 — GREASEWEAZLE READY WITHOUT HARDWARE:** tool detection, mocked backend, safe command construction, raw/derived artifact model.
- [ ] **M5 — GREASEWEAZLE LIVE:** raw-flux capture + decode/redecode + flux microscope after hardware arrives.
- [ ] **M6 — COMPLETE SUITE:** Office conversion, integrity, audit workbook/report exports, customer ZIP packaging.
- [ ] **M7 — HARDENING:** recovery regression tests, crash/cancel behavior, settings polish, release build.
- [ ] **M8 — ZERO-TOUCH PRODUCTION:** concurrent USB + Greaseweazle scheduler, automatic escalation, one-button downstream pipeline, 136-disk afternoon benchmark, and near-minimum operator touches.

## 20. First implementation session after repository is created

- [x] Read the new GitHub repository exactly as pushed.
- [x] Read BareEye / QuadBench / EagleCast reference files needed for eframe setup and DPI behavior.
- [x] Add dependencies/build-dependencies and Windows manifest support.
- [x] Split the hello-world project into the initial module skeleton.
- [x] Create the first real GUI shell.
- [x] Add project create/open and settings/tool-health structures.
- [x] Push the first checkpoint even if some planned pieces are still stubbed.
- [x] Then start the Windows read-only floppy backend immediately; do not spend three days making pretty cards before we can read a disk. :D
