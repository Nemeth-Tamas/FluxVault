# FluxVault — TODO

> Floppy archival, forensic imaging, recovery, conversion, audit, and customer-delivery suite.
>
> **Primary rule:** Source floppy media is read-only. FluxVault may write images, logs, extracted files, reports, and packages to the workstation, but it must never intentionally write to a customer floppy.

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

## 1. Safety invariants — must exist before real media testing

- [x] Create a central `MediaSafetyPolicy` / equivalent that marks all physical-floppy operations as READ ONLY.
- [x] Windows USB-floppy backend opens `\\.\A:` (or selected drive) with read access only; never request write access.
- [x] No code path may call a filesystem write operation against the floppy drive letter.
- [ ] Greaseweazle integration exposes acquisition/info/convert operations only.
- [ ] Never expose or invoke `gw write`, erase, clean, or another destructive Greaseweazle operation.
- [x] Show a persistent **SOURCE MEDIA: READ ONLY** indicator whenever a physical drive is selected.
- [x] Recommend the physical write-protect tab for customer disks when available.
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
- [ ] Non-blocking modal/dialog for “Insert floppy #NNN”.
- [x] Operator log panel with timestamps and copy button.
- [ ] Persistent per-job cancel button where cancellation is safe.
- [ ] Do not fake capabilities: controls for unavailable hardware/tools are disabled with a useful reason.

## 4. External-tool discovery

- [x] Tool manager detects/configures:
  - [x] 7-Zip (`7z.exe` / `7zz.exe` / `7za.exe`).
  - [x] LibreOffice (`soffice.com` preferred, `soffice.exe` fallback).
  - [x] Greaseweazle host tools (`gw.exe`) when installed later.
- [x] Store operator-selected paths in settings.
- [x] Show detected version and health check for each tool.
- [x] Provide a “Test tools” action.
- [x] Capture stdout/stderr and exit code for every external process.
- [ ] Kill a full conversion process tree on timeout, matching current converter behavior.

## 5. USB floppy acquisition MVP — **first working milestone**

This is the first “we can actually use FluxVault on customer media” target. It should replace the manual `FloppyArchiver_v1.5_manual.ps1` workflow before we chase fancy recovery features.

- [x] Enumerate/select floppy drives on Windows; A: must work with the current USB floppy reader.
- [x] Manual insertion/removal workflow. Do not depend on flaky automatic USB-floppy media detection.
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
- [x] Offer **Next floppy** while preserving manual operator confirmation.
- [ ] Add audible completion/error cues optionally (configurable).
- [ ] Test against several known-good disks and several damaged disks from the current batch.

### MVP acceptance test

- [ ] Insert a known-good 1.44 MB floppy -> FluxVault creates a 1,474,560-byte image, zero bad sectors, SHA-256, log, and project record without writing to source media.
- [x] Insert a known-bad floppy -> FluxVault completes a correctly sized image where possible, identifies exact unreadable sectors, preserves the partial status, and routes the disk to Recovery.
- [x] Re-run the same floppy -> creates/preserves a new attempt instead of destroying the previous evidence.

## 6. Existing archive/log compatibility

- [x] Parse current `FloppyArchiver` logs (`BEGIN`, `GEOMETRY`, retries, `BAD_SECTOR`, `SHA256`, `END`).
- [ ] Parse current DMDE Copy Sectors logs using the same **multi-pass/latest-sector-state-wins** rule as the PowerShell tooling.
- [ ] Recognize forward and reverse DMDE passes.
- [x] Preserve statuses for unfinished logs as **IN PROGRESS**, not “broken”.
- [x] Exact `NNN.log` must outrank auxiliary `NNN_scan.log`, retry-note logs, etc.
- [ ] Import existing `.bin`, `.img`, `.ima` images and ignore `.partial.*` files as completed acquisitions.
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
- [ ] Detect an operator-created recovery folder without the auto-extraction marker as **manual recovery present**.
- [ ] Preserve the current concept of an immutable first recovery backup (`Recovery/NNN/pass1` or equivalent).
- [ ] Build/update a project-wide recovered-file manifest.
- [ ] Route these cases to Recovery instead of pretending success:
  - [ ] non-clean image / unreadable sectors;
  - [ ] missing/unfinished/unrecognized acquisition log;
  - [ ] FAT listing failure;
  - [ ] extraction failure;
  - [ ] apparently readable image with zero recovered files when operator review is warranted.

## 8. Recovery workbench — pre-Greaseweazle

- [x] Recovery queue ordered by severity/attention state.
- [ ] Show current image, bad-sector count/list, source log, extraction result, prior attempts, and manual notes in one screen.
- [x] **Re-read with USB drive** action creates another immutable acquisition attempt.
- [x] Compare attempts sector-by-sector.
- [x] For 1-2 bad sectors, attempt evidence-based mirrored-FAT reconstruction into a separate derived image with per-sector provenance; never guess arbitrary bytes.
- [ ] Build an optional **best composite sector image** from multiple attempts, but only with a provenance map recording the source attempt for every replaced sector.
- [ ] Never destroy original attempt images when creating a composite.
- [ ] Allow import of a DMDE-recovered folder and DMDE log, replacing today’s “manually rescan then rerun scripts” dance.
- [ ] Immediately re-run extraction/audit state after a new recovery result is imported.
- [ ] Hex/sector inspector for selected sectors with LBA + CHS + attempt provenance.

## 9. Greaseweazle integration — hardware-independent groundwork

Greaseweazle host tools are intentionally wrapped rather than reimplemented initially. Current upstream supports Windows `gw.exe`, raw-flux formats including SCP/KryoFlux, and a separate `gw convert` path, so we can build/test command generation and output parsing before the board arrives.

- [ ] Create `GreaseweazleBackend` abstraction with a mock/no-hardware mode.
- [ ] Detect `gw.exe`, run info/version command, and show device status.
- [ ] Build commands as argument arrays, never shell-concatenated strings.
- [ ] Unit-test command generation without hardware.
- [ ] Parse `gw` stderr/stdout incrementally into GUI progress/events.
- [ ] Store full command, version, start/end time, exit status, and captured output for every run.
- [ ] Add “hardware not connected” UI state rather than error-spamming.

## 10. Greaseweazle raw-flux acquisition — after board arrives

- [ ] Detect board + connected drive and display device/firmware info.
- [ ] **Preservation capture defaults to true raw flux**, e.g. SCP/KryoFlux, not regenerated “perfect” flux.
- [ ] Important guardrail: if `gw read --format=...` is used for a raw-flux file, pair it with `--raw`; otherwise Greaseweazle may regenerate flux and fill undecodable sectors rather than preserving the physical capture.
- [ ] Default recovery workflow: first capture raw flux once, then perform as much decoding/re-decoding as possible from that preserved capture instead of repeatedly stressing fragile media.
- [ ] Allow configurable revolutions for raw capture where the selected image type supports it.
- [ ] Preserve every raw acquisition as an immutable attempt with SHA-256.
- [ ] Derive sector images from raw captures using `gw convert --format=<profile>`; derived images are separate artifacts, never replacements for raw flux.
- [ ] Profiles initially required for this collection:
  - [ ] IBM PC 1.44 MB / HD.
  - [ ] IBM PC 720 KB / DD.
- [ ] Later expose other Greaseweazle disk definitions without hardcoding the whole universe into FluxVault.
- [ ] Track/head selection and step settings available under **Advanced**, not in the basic happy path.
- [ ] Keep direct physical re-reads operator-controlled; no endless automatic hammering of a fragile disk.

## 11. “Mini electron microscope the shit out of it” flux recovery view

- [ ] Track/head map for raw-flux capture quality.
- [ ] Per-track decoded sector summary: present, valid CRC, bad CRC, missing, duplicates/unusual IDs where available.
- [ ] Compare multiple revolutions/passes visually.
- [ ] Show weak/problematic regions and which decode attempt recovered each sector.
- [ ] Re-run decode from the same raw flux with alternate Greaseweazle profile/settings without touching the physical disk.
- [ ] Compare results from USB-sector reads versus Greaseweazle-derived sector images.
- [ ] Composite/reconstruction tools must retain provenance and never masquerade reconstructed bytes as an untouched original capture.
- [ ] Export a recovery note describing what was physical capture, decoded data, retry-recovered data, and reconstructed/composited data.

## 12. Legacy Office conversion pipeline

Reproduce `Convert-LegacyOffice_v4_Timeout_Audited.ps1` behavior inside the app workflow.

- [ ] Mirror recovered originals into customer-facing `Converted` paths without changing the forensic source tree.
- [ ] Remove DMDE artifact path segments from delivery paths (`$Noname`, `$Root`, raw-signature folders) while preserving forensic path mapping.
- [ ] Resolve name collisions deterministically (`[recovered copy N]`).
- [ ] Preserve recovery-method labels: normal filesystem, DMDE filesystem recovery, signature recovery.
- [ ] Support current source extensions/plans:
  - [ ] Word-family -> DOCX + PDF: `.doc`, `.rtf`, `.wps`, `.wri`, `.wpd`, `.sdw`.
  - [ ] Spreadsheet-family -> XLSX + PDF: `.xls`, `.xlw`, `.xlt`, `.wk1`, `.wk3`, `.wk4`, `.wks`, `.123`, `.wb1`, `.wb2`, `.wq1`, `.wq2`, `.sdc`.
  - [ ] Presentation-family -> PPTX + PDF: `.ppt`, `.pps`, `.pot`, `.sdd`.
- [ ] Configurable worker/thread count.
- [ ] Per-output timeout (default 45 s to match current workflow).
- [ ] Process-tree kill on timeout.
- [ ] Skip/reuse already-valid outputs unless force reconversion requested.
- [ ] Validate generated Office OOXML as ZIP containers with required internal files.
- [ ] Validate generated PDFs via `%PDF-` header + `%%EOF` tail sanity check.
- [ ] Record `OK`, `PARTIAL`, `FAILED`, `TIMEOUT`, and `REUSED` results plus details/duration.
- [ ] Conversion issues page with retry selected / retry failed actions.

## 13. Audit/report engine

Replace the current updater/audit script chain with one in-app source of truth while keeping export compatibility.

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
- [ ] Audit must be re-runnable/idempotent and never alter source floppy media.

## 14. Customer package builder

Reproduce `Make-FloppyCustomerPackage_v1.ps1` in the GUI.

- [ ] Choose destination outside project/source tree and enforce that guardrail.
- [ ] Stage only allowed archival/customer folders.
- [ ] Exclude internal helper/state files from customer content.
- [ ] Include selected customer-useful reports.
- [ ] Generate package manifest with size, timestamp, SHA-256.
- [ ] Generate manifest SHA-256 file.
- [ ] Generate README explaining Images / Logs / Extracted / Converted / Recovery / Reports and known limitations.
- [ ] Create timestamped ZIP.
- [ ] Hash final ZIP and write `.zip.sha256`.
- [ ] Verify ZIP inventory/count/total bytes against staging before declaring success.
- [ ] Optional “keep staging folder” setting.

## 15. Current dataset regression targets

Use the supplied `FloppyFinalReport.xlsx` and existing archive as regression truth while porting functionality.

- [ ] Import/represent all 136 floppy records.
- [ ] Current reference summary: 136 images present; 94 imaging OK; 42 imaging not OK; 86 floppies fully OK; 50 need attention/are partial.
- [ ] Reproduce 1,667 recovered source-file records and the current conversion/audit counts when pointed at the same archive contents.
- [ ] Correctly represent severe cases rather than assuming every image is 1.44 MB; current data includes manually recovered/high-error cases and at least one 417,792-byte image.
- [ ] Regression-test examples with 1 bad sector, tens of bad sectors, hundreds of bad sectors, conversion-only failures, no-recovered-file cases, manual recovery, and signature recovery.

## 16. Testing

- [ ] Unit tests for floppy-number parsing and zero-padding.
- [x] Unit tests for legacy archiver-log parsing.
- [ ] Unit tests for DMDE multi-pass map replay (later successful `C` replaces earlier `E`).
- [ ] Unit tests for path cleanup / delivery naming / collision handling.
- [ ] Unit tests for Greaseweazle command construction, especially raw-flux safety flags.
- [ ] Unit tests for project persistence and migrations.
- [ ] Unit tests for SHA/integrity helpers.
- [ ] Fixture-based tests using scrubbed/sample logs and tiny synthetic images; never require a customer floppy for automated tests.
- [x] Integration test for 7-Zip adapter.
- [ ] Integration test for LibreOffice adapter when installed.
- [ ] Greaseweazle hardware tests marked/isolated so normal `cargo test` works without hardware.

## 17. Milestones

- [x] **M0 — Skeleton:** eframe window, DPI fix, module layout, settings, project open/create, worker/event plumbing.
- [x] **M1 — WORKING USB ARCHIVER:** safely image a real floppy, retry/fallback, bad-sector map, SHA-256, persistent project record.
- [ ] **M2 — SCRIPT REPLACEMENT CORE:** import legacy archives/logs, auto extraction, recovery queue, manifests.
- [ ] **M3 — RECOVERY WORKBENCH:** multiple USB attempts, compare/composite with provenance, DMDE import workflow.
- [ ] **M4 — GREASEWEAZLE READY WITHOUT HARDWARE:** tool detection, mocked backend, safe command construction, raw/derived artifact model.
- [ ] **M5 — GREASEWEAZLE LIVE:** raw-flux capture + decode/redecode + flux microscope after hardware arrives.
- [ ] **M6 — COMPLETE SUITE:** Office conversion, integrity, audit workbook/report exports, customer ZIP packaging.
- [ ] **M7 — HARDENING:** recovery regression tests, crash/cancel behavior, settings polish, release build.

## 18. First implementation session after repository is created

- [x] Read the new GitHub repository exactly as pushed.
- [x] Read BareEye / QuadBench / EagleCast reference files needed for eframe setup and DPI behavior.
- [x] Add dependencies/build-dependencies and Windows manifest support.
- [x] Split the hello-world project into the initial module skeleton.
- [x] Create the first real GUI shell.
- [x] Add project create/open and settings/tool-health structures.
- [x] Push the first checkpoint even if some planned pieces are still stubbed.
- [x] Then start the Windows read-only floppy backend immediately; do not spend three days making pretty cards before we can read a disk. :D
