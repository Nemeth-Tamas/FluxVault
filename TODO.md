# FluxVault — TODO

For operating the current build, start with [README](README.md), [the beginner/advanced tutorial](TUTORIAL.md), or [the quick cheat sheet](CHEATSHEET.md). This checklist includes future commands and goals; it is not an instruction sequence to run.

> CLI-first floppy archival, forensic imaging, recovery, conversion, audit, and customer-delivery suite. Work from a project folder in PowerShell using `fluxvault` commands; there is no desktop GUI.
>
> **Primary rule:** Source floppy media is read-only. FluxVault may write images, logs, extracted files, reports, and packages to the workstation, but it must never intentionally write to a customer floppy.
>
> **Product target:** FluxVault is an automated archival appliance, not a collection of expert-only recovery tools. Except for physically inserting, removing, or moving a floppy between drives, the normal operator workflow should require no recovery decisions, no manual DMDE work, no hand-edited spreadsheets, no manual extraction, and no manual report/package assembly. The production loop is now implemented: **`fluxvault production start --last-disk N` -> insert/confirm/swap -> automatic finishing and verified archival ZIP**. `production resume` restores saved options; partial/raw-only results retain attention, not customer-certification. Single-GW is default, USB + GW is opt-in. [Production guide](PRODUCTION_WORKFLOW.md). Additional formats, broader damaged-document reconstruction and measured full-collection acceptance remain separate targets.
>
> **Throughput target:** With one USB floppy drive and one Greaseweazle-connected drive operating concurrently on different disks, a 136-disk mixed-condition job—including automatic verification, escalation, extraction, conversion, audit, and ordinary recovery passes—should be achievable within one operator afternoon (target: no more than roughly 6 hours of attended wall-clock time, excluding genuinely pathological media that must continue unattended or be reported as unrecoverable).

> **GW A/B scope revision (2026-10-09):** Simultaneous independent capture on two drives attached to one GW remains out of scope. The separate opt-in alternating/preloaded A/B concept is **deferred until a suitable two-headed ribbon is available**, per the operator's latest request; retain its requirements in section 11, but do not prioritize implementation now. Ordinary single-GW runs still default to B (straight cable), and existing `--double` means one USB + one GW. A possible second USB drive is not yet confirmed; two-USB + one-GW `--trio` remains a separate opt-in future extension with independent USB identities/reservations/receipts. No new multi-drive mode is implemented. The Samsung remains unqualified (possible track misalignment); qualify drives/cabling with read-only reference tests before accepting interchangeable captures.
>
> **Soft delivery target:** By **2026-10-15**, finish most core automation/production functionality and its practical validation. Final polish may follow afterward. These are priorities and targets, not completed capabilities or an unattended work schedule.

> **Greaseweazle-only priority (2026-10-06):** USB is optional. On the working Mitsumi drive (straight cable, selector B), start fast, decode preserved raw evidence, and escalate only problem areas within time/media-stress limits. Routine drive/ribbon swapping is not part of the workflow. `greaseweazle recover N` implements bounded single-disk recovery; `greaseweazle scan` adds numbered/Enter swap confirmations, resumable numbering and continuous saved-image processing. Standard IBM format discovery and raw-only ambiguity continuation are implemented; additional decoders, deeper damaged-filesystem recovery and live production acceptance remain open. See `CHAT_TO_CHAT_GREASEWEAZLE.md` for the hardware handoff and `GREASEWEAZLE_PREFLIGHT.md` for live results.

> **Native recovery checkpoint (2026-10-06):** Saved partial FAT12 images now automatically yield independently intact files through native readable-chain extraction. The saved WinWord 1 capture produced 22 forensic files / 1,208,710 bytes, with unchanged source hash and the unreadable sector still flagged. Native generation 2 validates long names, preserves ASCII short-name case flags, and excludes identified Windows metadata from new delivery plans (21 installer originals). This replaces manual extraction for that bounded case, not full DMDE capability: missing boot/directory reconstruction, deleted/orphaned chains, carving, automatic format discovery and production scheduling remain open.

> **First 20-disk pilot completed (2026-10-06):** Customer 001–020 saved with 17 clean results and three partials (005/012/017, one missing sector each, no conflicting sectors). All 20 reached extraction; all 172 conversion jobs succeeded. The operator approves number-only scanning and requests much more visible swap cues. Next priorities: terminal visibility, saved/configurable scan conversion workers with balanced scheduling, and lossless background raw-capture compression. See `PILOT_20.md` for measured timing/storage and the extraction caveats; this is not yet full script/DMDE yield equivalence or customer-delivery certification.

> **Offline preparation checkpoint:** Visible/colored cues, balanced configurable conversion workers and Enter-only confirmations are shipped. New scans now add bounded IBM format discovery and verified background packing. A copy of all 26 captures shrank 76.42%, with 20/20 disks hash-healthy and packed 007 decoding identically. Original pilot raw files remain unchanged. See `PROGRESS.md`; full script/DMDE yield equivalence and production acceptance remain separate targets.

> **Damaged-filesystem starter:** Native generation 3 can infer an explicitly warned standard 720 KB/1.44 MB layout when boot metadata is missing, only with corroborating readable matching FATs/root file chains. It never synthesizes boot/file bytes or certifies inferred geometry. Skipped files retain exact holes/unmapped tails. Four saved pilot images retain 34 payload hashes under simulated boot loss; 017 is conservatively refused without its readable BPB. Historical 009's live JPEG has 4,608 unreadable bytes, so no complete payload is exported. See `DAMAGED_FILESYSTEM_RECOVERY.md`.

> **Automatic carving checkpoint (2026-10-07):** Generation 4 adds terminated allocated orphan-chain recovery, structurally validated signatures and independently readable embedded objects from partial files, with exact offsets/FAT evidence, immutable inventories and separate signature-delivery labels. Known deleted/free allocation stays excluded when readable layout exists; raw fallback explicitly cannot determine live/deleted status. Isolated saved 021–032 yielded 11 additional candidates on 023/024/027; both 024 Word containers successfully became DOCX/PDF, and all 39 Office conversions passed. Historical byte matching remains 38/89, not an invented parity gain. See `CARVING_RECOVERY.md`.

> **Deeper recovery checkpoint (2026-10-08):** Native generation 5 automatically reconstructs sufficiently anchored lost-parent directories, retains structurally validated single-missing-FAT-link suffix alternatives and salvages mapped main-text segments from incomplete binary Word documents. Word-text engine 2 selectively bypasses unrelated missing compound-directory entries and creates offline readable HTML editions with explicit gaps. Originals/earlier generations remain intact; guessed links/text are never certified. Saved 021–032 retains all 64 payloads; 024 adds 1,053 readable main-text positions and 027 now yields 25,672 with a 256-position gap. Saved 017 adds 3,229 positions from its incomplete Word file. Text/editions are separate forensic evidence, not repaired DOCs or extra whole-file yield. See `DEEP_RECOVERY.md`.

> **Continuous saved-file processing:** New scans enqueue verified acquisitions for coalesced background extraction/recovery/conversion/audit/workbook work; swap cues remain unobscured. Durable jobs, source verification, whole-project ownership, short publication snapshots, shared CPU/RAM/volume-space admission, status/resume and final reconciliation are implemented. Old journals retain tail mode. See `BACKGROUND_PROCESSING.md`; physical contention/throughput and full 136-disk acceptance remain distinct validation gates.

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

- [x] The default documented CLI workflow is a guided production queue, not a chain of expert-only commands. `production start` owns feeding through checked archival packaging; old expert `scan` stays compatible.
- [x] The operator's only routine responsibilities are placing/removing disks, moving a disk from USB to Greaseweazle when prompted, and optionally entering a physical label or note. Supported production jobs run bounded recovery/file processing/reports/packaging automatically; fatal hardware/integrity exceptions still require attention, not invented success.
- [ ] FluxVault automatically chooses retries, read direction, composite inputs, extraction strategy, recovery escalation, conversion, audit, and packaging policy from recorded evidence.
- [x] Expert subcommands/flags remain available, but normal supported production jobs do not require understanding sectors, FAT, DMDE, flux, profiles, hashes, or conversion filters. A fresh range endpoint and physical confirmations suffice; saved options resume automatically.
- [ ] Every automatic decision records its evidence, confidence, limits, and provenance so automation never hides guessing or fabricates recovered data.
- [ ] A disk may finish as **verified**, **partially recovered**, or **unrecoverable within policy**; the program must not block the entire batch waiting for manual repair.
- [ ] All stages are resumable after application restart or workstation failure without repeating completed evidence-preserving work.
- [x] No normal production prompt asks for a technical choice the program can derive safely. Default format/policy/worker/packing/recovery choices remain automatic; physical identity, protection and reseating are unavoidable operator actions. Expert overrides remain optional.

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
- [x] Model each floppy as a stable record with zero-padded number (`001`, `002`, ...), label/notes, acquisition attempts, current preferred image, extraction state, recovery state, conversion state, and audit state. `disk show` now projects this lifecycle over immutable attempts and saved controls; `disk note` retains optional annotation history. Preference is automatic evidence ranking; explicit operator selection remains open below. Historical reports are not fresh certificates. [Closure evidence](CHECKLIST_85.md).
- [x] Model acquisition attempts as immutable records: source backend, timestamp, geometry/format, output artifacts, hashes, bad-sector map, status, and log path.
- [ ] Allow one attempt to be marked **preferred/current** without deleting older attempts.
- [ ] Preserve enough provenance to answer: “Which read/pass produced this sector/file?”
- [x] Import an existing script-created archive as a project without forcing re-imaging. `project import --source ZIP --destination NEW_FOLDER` preserves every original member, checks CRC/copied/source hashes, creates metadata/cursor and publishes only into a fresh destination; offline preview, exclusive destination ownership and retained failed stages are tested. Original 136-image ZIP passes independent member comparison. Legacy claims remain attention; no managed recovery/Office provenance is invented. [Usage and limits](LEGACY_IMPORT.md).

## 3. CLI operator experience

- [x] No-argument `fluxvault` shows help instead of opening a window.
- [x] Folder-first project discovery plus `init`, `status`, disk, drive, recovery, extraction, conversion, audit, report, package, and tool commands.
- [x] Saved command logs and stderr progress replace the window's status/operator-log panel.
- [x] **Unmistakable swap/action banners.** Separate physical completion from downstream processing, prominently display `DONE 004 / REMOVE 004 / INSERT 005`, and explicitly say when waiting for the operator rather than silently appearing busy.
  - [x] Add semantic terminal colors: green clean completion, red partial-saved swaps with explicit safe-to-proceed text, amber warnings/raw-only exceptions, cyan next physical action, red operation failure without number advancement. Always include plain-text labels and disk/station identity; never rely on color, special glyphs, or animation alone.
  - [x] Show temporary interactive read/recovery loading progress with ASCII heartbeat, elapsed time and unique host-reported track visitation, resetting targeted ranges without inventing sector-yield percentages. Clear before diagnostics and success/error swap cues; no animation in redirected stderr/JSON or dumb terminals. Test cleanup, quiet heartbeat, duplicates/range bounds and forced-color redirection.
  - [x] Support automatic terminal color detection, explicit color override, and `NO_COLOR`; readable ASCII/monochrome fallback on Windows, no automatic ANSI escapes in redirected logs or JSON/stdout. `--color always` explicitly forces stderr decoration; unit/executable tests cover plain/color/JSON output modes.
  - [x] Show concise reading/decoding/processing/waiting status with elapsed time; preserve a clear next-action cue when concurrent worker messages arrive. Optional configurable audible cues may supplement, not replace, the banner. Existing recurring dual ACTION lines/track meters and bounded station-specific sound cues now join unified saved-only `status`; original feeding-console custody remains authoritative.
    - [x] Dual reader status shows elapsed time/latest capture-decode progress and explicit per-station ACTION lines; a recurring refresh survives frequent worker output. Pending USB transfers include held saved partials without conflating them with removal-confirmed queue entries. Offline status prints concise custody actions and does not mistake persisted READING for proof of an active reader. Session reports add invocation-local feed/full elapsed times; regression and executable tests cover safe status and JSON/monochrome behavior.
  - [x] At the configured endpoint print `BATCH FINISHED / REMOVE 020`, then downstream progress/results; distinguish a persisted next cursor of 021 from an instruction to insert 021. Show session totals and whole-project totals separately.
- [x] Add explicit testing convenience `scan --no-verify`: Enter confirms the displayed disk, with a prominent warning at every custody prompt. Skip label typing only, never source read-only access, evidence/hash verification, numbering/resume safeguards or per-disk confirmation. Record mode in telemetry; do not persist the shortcut into future scans. EOF never starts a read, wrong explicit numbers remain refused, and default blank-input refusal is retained.
- [x] `greaseweazle preview` preserves the former safe-command mock/preview without touching hardware.
- [x] `greaseweazle info` exposes the audited read-only device/firmware query through the CLI; live V4.1/firmware 1.6/Mitsumi-B operation validated on 2026-10-05.
- [x] Make the default command path much shorter: one production command runs the full safe chain with plain-language status and next physical action. `production start --last-disk N` / `production resume` span scan, recovery, processing, audit and verified archival ZIP under one owner/controller; unfinished labels/transfers prevent packaging.
- [x] Print concise station-specific prompts: “Insert floppy #NNN in USB”, “Move floppy #NNN to Greaseweazle”, or “Archive floppy #NNN and insert #NNN+1”. Existing uN/gN custody/saved/transfer cues and endpoint/no-index prompts are exercised by live pilots and executable contracts; saved dashboard reuses those station actions.
- [x] Add safe pause/resume/cancel semantics and a persisted job queue; a terminal closing must not silently lose completed evidence. Supported controllers/queues preserve sealed receipts, pending custody, raw/Office/package partials and require reconfirmation. Actual mock process-exit/stop fixtures and live WinWord stop/resume pass; arbitrary power-loss/publication cutpoints remain separate open requirements.
- [x] `status` should show both stations, queued escalation, throughput, estimated time, current operation, and the next physical action. New additive versioned `operations` view combines saved station/controller/production/backlog records and rough historical fresh-feed ETA. Unknown/live-reader state and conflicting inactive modes are not guessed; full recovery/conversion-tail ETA remains open in section 11.
- [x] Unavailable hardware/tools must be reported with an actionable reason; never imply a planned capability already works. Tool preflight/configuration errors, absent-board/protection refusal, bounded No Index reseat prompts and preserved raw-only format exceptions have explicit action/error contracts; no hardware availability is inferred by offline status.

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
- [x] Add optional per-invocation `scan --sound on|off` completion/error cues for single-GW, USB-only and dual scanning. Default silent; station/outcome-specific short Windows tones run in one bounded non-blocking worker, never contaminate redirected JSON/logs, change acquisition policy, or replace written custody/swap instructions. Busy/stale/shutdown notices may be dropped; audio availability is not guaranteed. Parser, mock scan/redirection, worker pressure/cancellation and real PowerShell-completion tests cover the interface without playing audio or touching media.
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
- [x] Import existing hashes and current archive index where possible. Original CSV/log bytes are retained; indexed/logged image hashes are compared with independently measured copied bytes, and unmatched rows/status totals/discrepancies remain explicit in LegacyImport.json. No old hash/claim is silently rewritten or newly certified.
- [x] Display legacy/manual recovery state without requiring the old Excel workbook. Plain-text LegacyImport.txt and detailed JSON report each saved image's old status/size/bad-sector claim, extracted/converted/recovery folder counts and discrepancies. Counts include auxiliary files, not certified recovered-file yield; ordinary legacy disk/status inspection remains compatible.

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
  - [x] Preserve earlier native generations during engine upgrades: publish versioned outputs/reports and verify reuse. Shared preference now compares verified byte multiplicity, reachable-file coverage and validated names within the selected acquisition; a newer version alone cannot hide a richer earlier result.
- [x] Emit `RecoveryExceptions.txt` alongside legacy `BrokenForDMDE.txt`; unresolved cases are exceptions, not instructions to do manual DMDE as the default workflow.
- [x] Route these cases to Recovery instead of pretending success:
  - [x] non-clean image / unreadable sectors;
  - [x] missing/unrecognized acquisition log;
  - [x] unfinished acquisition log is preserved separately as **IN PROGRESS**;
  - [x] FAT listing failure;
  - [x] extraction failure;
  - [x] apparently readable image with zero recovered files when operator review is warranted.
- [x] Automatically run extraction after an eligible acquisition or newly derived preferred image; no separate action in production mode. Coalesced background jobs plus final reconciliation use the existing replay-verified offline recovery handoff. Immediate means scheduled automatically, not that CPU work blocks the next physical read.
  - [x] Single-disk `greaseweazle recover N` dispatches the existing project-wide extraction/conversion/audit/workbook chain by default; partial images still follow conservative extraction eligibility rules.
  - [x] The retired GUI queued image-only extraction after each clean USB acquisition; the CLI retains `extract disk N`/`extract all`, but automatic nonblocking post-scan extraction remains a production-scheduler task.
  - [x] Add a one-button offline project pass that batches eligible extraction, conversion, evidence audit, and a Hungarian workbook without touching physical media.
- [ ] Automatically re-run extraction and file inventory whenever a better composite, decoded flux image, or reconstructed filesystem becomes preferred.
- [ ] Replace “operator review required” as the normal next step with a bounded automatic recovery plan; operator review is the final exception state only.
- [x] Treat existing manual recovery folders/DMDE imports as legacy compatibility inputs, not as the intended recovery workflow. Production uses native automatic recovery/extraction, retains operator folders and supports guarded legacy import; this is not a claim of equal DMDE yield on all damaged media.

## 8. Automated recovery engine — pre-Greaseweazle

- [x] Recovery queue ordered by severity/attention state.
- [x] Add a comprehensive `disk show N --details` view (and JSON equivalent) for current image, bad-sector list, source log, extraction result, prior attempts, automated decisions, and optional operator notes. Attempt fields remain compatible; versioned `lifecycle` adds automatic selection, extraction/recovery decisions, processing jobs, conversion/audit association and retained notes. Stale/foreign/oversized controls refuse or remain explicitly historical. Real saved 059 confirms three extracted files/two recorded Office jobs/five unresolved sectors without changing source controls.
- [x] **Re-read with USB drive** action creates another immutable acquisition attempt.
- [x] Compare attempts sector-by-sector.
- [x] Reconstruct readable mirrored-FAT sectors into a separate derived image with per-sector provenance even when the disk has more than two bad sectors; other sectors remain unresolved and are never guessed.
- [x] Build an optional **best composite sector image** from multiple attempts, but only with a provenance map recording the source attempt for every replaced sector.
- [x] Reject composite sources whose recorded image hash changed or whose mutually readable sectors disagree; reject duplicate attempt IDs and unsafe source file types.
- [x] Add a read-only per-disk recovery plan (`fluxvault recovery plan`) that re-hashes saved attempts and ranks composite, mirrored-FAT, and physical reread/flux candidates without writing to source media.
- [x] The one-button offline `process` workflow automatically creates or verifies/reuses mirrored-FAT derived images where the saved-image plan finds redundant readable sectors; supported results now enter extraction/delivery as explicitly DERIVED catalog attempts, with unresolved data retained.
- [x] The offline `process` workflow automatically attempts provenance-tracked composites when saved attempts have complementary readable sectors, reuses verified prior results, and declines conflicting captures without stopping other disks. It can then apply mirrored-FAT repair to a still-partial composite and automatically hand supported improved images to native extraction/audit.
- [x] Persist each offline recovery decision and exception in `Reports/OfflineRecoveryDecisions.json` for audit/customer-package context; this is not a claim that unresolved sectors were recovered.
- [x] Never destroy original attempt images when creating a composite.
- [x] Legacy/fallback compatibility: allow import of a DMDE-recovered folder and DMDE log. This must not remain part of the intended normal workflow.
- [ ] Immediately re-run extraction/audit state after a new recovery result is imported.
- [x] Hex/sector inspector for selected sectors with LBA + CHS + attempt provenance, available as an advanced diagnostic rather than a required workflow step. `recovery sector N --lba L [--sectors 1..8] [--attempt N] [--json]` reads completed native images only, verifies snapshot/hash/map bindings, labels placeholders/unknown/derived evidence, replays offline donor/FAT-copy origins and distinguishes recorded flux provenance from independent replay. No media, tools or project writes. See `SECTOR_INSPECTION.md` for bounds and legacy limitations.
- [ ] Build an automatic recovery policy engine that selects the next safe action from evidence and stops at configurable media-stress/time limits.
- [ ] Automatically combine all USB attempts, retry-recovered sectors, reconstructed FAT copies, and later flux-derived sector images into the best provenance-tracked derived image.
  - [x] Automatically publish supported complete-map saved-image composites and mirrored-FAT results into the image catalog for extraction/manifest/conversion/audit, with replay-verified original source/map/sector provenance, immutable reserved attempts, manual-folder preservation and explicit DERIVED attention. Repeated processing reuses the same result; conflicting/oversized/unbound sources are declined. Synthetic chain/boundary/tamper checks and isolated saved HD/DD payload equivalence pass. See `OFFLINE_RECOVERY_HANDOFF.md`; arbitrary raw/decode donor integration, real publication process-kill tests and broader format/geometry support remain separate work.
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
    - [x] Recover allocated lost-parent directory trees using readable dot/self/parent and surviving child anchors, agreeing terminated FAT chains and independent namespace/ownership checks. Mark reconstructed roots/names and unknown historical ownership; record gaps, refuse known deleted/shared/conflicting/cyclic components and retain source/FAT evidence. Nested/non-adjacent file and fragmented-directory-gap fixtures pass. No missing entry or original root name is invented.
  - [x] Use validated long names to identify/exclude Windows OS metadata from new delivery plans/packages, while keeping forensic extraction/inventory; never guess that every `SYSTEM~1` alias is `System Volume Information`.
  - [x] Automatically quarantine obsolete, hash-proven program-owned ORIGINAL delivery mirrors when naming/source generations improve: retain equivalent payload multiplicity within the same disk, verify replacements, preserve edited/untracked/needed/conflicting copies and audit each retirement. Ownership snapshots and restartable intents move originals without overwrite into `Recovery/DeliveryQuarantine`; forensic extraction stays intact. Pre-ledger copies are not retroactively claimed; old Office derivatives remain preserved. See `RECOVERY_SELECTION.md`.
- [ ] Use both FAT copies, boot-sector/BPB evidence, root-directory entries, cluster chains, file sizes, and cross-attempt sector provenance to reconstruct damaged filesystems without arbitrary byte guessing.
- [x] Add evidence-supported orphaned cluster-chain recovery, clearly labeling confidence and recovery method. Generation 4 follows unclaimed terminated allocated chains in logical order, excludes known live/deleted ownership, refuses FAT disagreement/shared tails/cycles, splits at unreadable sectors and retains FAT-link/raw-extent provenance. Unknown original ownership/length is explicit; unsupported payloads remain unresolved.
- [x] Add explicit opt-in deleted-file recovery (`recovery extract N --include-deleted`) to saved-image processing; OFF by default, including new-project scan policies. Surviving unambiguous chains and independently validated all-free contiguous hypotheses publish immutable forensic-only results under Recovery, never normal live extraction/conversion. Raw deleted entries, offsets, hashes, exact extents and refusal reasons survive restart; source/manual/live outputs remain intact. Erased fragmented chains and deleted-directory reconstruction remain outside this bounded feature. DMDE previously included deleted recovery; it is not required for default delivery. See `DELETED_AND_FRAGMENT_RECOVERY.md`.
- [x] Add signature-based file carving as an automatic fallback for unreconstructable filesystems, preserving raw offsets and labeling filenames/paths as reconstructed. Bounded image-only runs, immutable generation/report/source bindings, rejected-candidate evidence, deduplication and automatic partial audit/delivery separation are implemented; no usable candidate is a reusable zero-file attention outcome.
- [x] Detect common document/archive/image signatures and validate carved outputs before including them in customer delivery. PNG/JPEG/BMP/GIF pixel decoding, PNG CRCs/all GIF frames, ZIP member decompression/CRCs and safe identities, RTF structural envelopes, strict OLE FAT/directory/mini-stream reading; Word/Excel candidates use normal Office conversion. Structural checks do not certify original semantics; unsupported formats remain open.
- [ ] Try multiple evidence-ranked interpretations automatically and retain all non-destructive candidates; never require the operator to choose a sector manually.
  - [x] For one unavailable FAT-link boundary, automatically test bounded allocated unclaimed suffixes of the exact remaining cluster count against supported format validation. Require a candidate spanning the known prefix into the tail; retain competing structurally valid alternatives under Fragment-Hypotheses with actual extents/links and the unknown link explicit. Exclude known deleted/shared/owned/bad data, preserve older generations and refuse to certify historical association. Arbitrary multi-boundary/erased fragmentation remains open.
- [ ] Reconstruct usable editable documents across broader damaged container/allocation formats without fabricating content; support bounded evidence-ranked structural repair and keep all repaired copies separate from originals. Missing CFB metadata, legacy non-OLE Word, other document formats, formatting/objects and semantic completeness remain unresolved.
  - [x] Automatically salvage readable main-text segments from incomplete supported Word binary DOC/DOT using validated CFB/FIB/CLX mappings, exact character-position gaps/source extents and strict metadata/UTF-16 checks. Preserve separate immutable UTF-8/report evidence under Recovery, exclude it from normal whole-file counts/Office conversion and expose `recovery documents N`; missing required metadata/encryption/unsupported formats are explicit refusals, not dictionary guesses. Saved 017/024 text and synthetic missing-text-sector cases validate the bounded feature; no repaired original claimed.
  - [x] Selectively map supported CFB v3/v4 Word/table regular/mini streams past unrelated unavailable nested directory entries, requiring a fully readable root sibling path and recorded allocation links; refuse detached-name substitution, required metadata holes, cycles, duplicate root identities and known allocation/metadata crosslinks. Preserve the strict refusal and exact metadata evidence without rewriting/certifying a compound container. Saved 027 now yields 25,672 mapped text positions with a 256-position gap.
  - [x] Automatically publish one bounded offline readable HTML edition per salvaged document with escaped text, visible segment boundaries/gaps, provenance/warnings and no active content/remote assets. Return paths in `recovery documents`/JSON, hash/reverify immutable editions, archive as forensic evidence only and retain old Word-v1 outputs; no manual text reassembly or whole-file/conversion inflation.
- [ ] For disks with hundreds of bad sectors, recover every independently verifiable file/fragment possible, then produce a precise unrecoverable-range report instead of failing the entire disk.
  Bounded progress: a 400-bad-sector fixture salvages its independent supported candidate; mapped readable portions of incomplete live files yield validated embedded objects and now automatically preserve raw readable fragments separately under Recovery. Exact logical offsets, physical extents, missing ranges/unmapped tails, EOF clipping, source hashes and immutable reuse are retained; ambiguous ownership is refused. Saved 021–032 preserved nine fragments / 449,539 bytes without increasing complete-file counts. Unknown layouts/entries, additional formats and exhaustive recovery remain open.
- [x] Automatically prefer a more complete recovery within the selected acquisition while retaining prior results and explaining why the preferred result changed. Shared selection uses verified payload multiplicity/reachable ownership/validated names, refuses tampered or mismatched evidence, preserves regressions/incomparable candidates and publishes immutable reasons. Manifest/conversion and extraction-presence consumers share this selector. Different acquisitions retain existing sector-map ranking; cross-acquisition file unions/deeper layout interpretation remain separate work. Saved 021–032 migration fixture promotes 53 reachable originals to 64 including carving and reuses all 64 after restart, without changing image hashes.
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
- [x] Apply bounded single-disk automatic physical-read policies based on missing/conflicting sectors, elapsed time, revolutions, and prior improvement; stop rather than endlessly hammering media. Default 4 stages/**600 seconds per stage**/2 consecutive completed non-improving passes; validated policies can tighten/change ceilings or explicitly request a whole-job limit.
  - [x] `greaseweazle recover N` works without USB: Fast whole disk, then fixed-profile problem-cylinder rereads with clean control cylinders; preserves raw/decode attempts and publishes immutable compatible image/log/metadata plus per-sector confidence/provenance.
  - [x] Persist stages and completed result; resume saved raw decode without a new read, verify completed hashes, reject changed policy/settings, block duplicate per-project/disk jobs.
  - [x] Live targeted recovery retained 2,879/2,880 sectors, no byte conflicts, and stopped with persistent LBA 24 explicitly unreadable. Repeat invocation read no media; downstream backup/audit/workbook ran.
  - [ ] Add cross-project physical-device reservation and interruption tests at every journal/publish boundary before calling this a production scheduler.
    - [x] CLI `scan`/`recover`/`capture`/`info` reserve one Greaseweazle across projects sharing the per-user settings directory; mock child-process contention and termination/release tests pass. Direct host/library calls and separate users are outside this guard.
    - [x] Test guided-scan restart before/after the cursor commit, custody reconfirmation after acquisition failure, changed output refusal, and Windows metadata commit failure without truncating the old project file.
    - [ ] Exercise every acquisition-journal/publication interruption boundary and independently validate multi-disk hardware behavior.
    - [x] Bind active host-process lifetime to the controller: Windows controller/operation jobs cover GW, probes, extraction and Office hosts before execution. Actual mock CLI capture termination in single/dual mode stops descendants, preserves partial evidence/custody and resumes exactly once; forced decode restarts offline without rereading. Held pipes, bounded tree timeout and suspended-start refusal also tested. This does not certify physical motor idle or every publication boundary. See `PROCESS_SUPERVISION.md`.
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
- [x] Tell the operator exactly when to move a USB-problem disk into the Greaseweazle drive and when it can be removed; no flux expertise should be required. Dual saved-partial/held/removal-confirmed queue states produce exact numbered MOVE/SET ASIDE/WAIT/REMOVE actions; gN reconfirms identity and saved completion seals precede removal cues. Status never promotes a stale reading record into safe-removal permission.

### Raw-capture storage efficiency — requested after the 20-disk pilot

The pilot holds 26 immutable SCP captures (20 initial plus six targeted rereads), totaling 985.13 MiB; the whole working project is about 1.08 GiB. Flux preserves multi-revolution timing evidence that sector-only script images did not contain. Storage optimization must not discard that added evidence or substitute regenerated flux.

- [ ] Benchmark lossless capture compression on representative clean, damaged and targeted captures; report ratio, CPU/RAM use and elapsed time rather than promise a fixed saving. Compare equivalent folder/package contents with the old archive, not an uncompressed working project against a ZIP.
  - [x] Add image-only `storage benchmark N`: use the largest hash-verified complete capture (128 MiB bound), compare ZIP/Deflate levels 1/6, measure compression/decompression and require identical size/SHA-256; work in bounded memory without writing/removing evidence.
  - [x] Validate saved customer 007: 54,050,828 bytes -> 12,343,912 bytes at level 6 (77.16% saving), roughly 0.86 s compression / 0.07 s roundtrip verification on this workstation. This is one capture, not an aggregate guarantee; broaden samples and shared CPU/RAM budgeting before automatic compression.
  - [x] Build/hash-verify the real 20-disk archival package with all 26 captures: raw flux compresses from 985.13 to 232.24 MiB (76.42% saving); the complete 937-file ZIP is 266.19 MiB. Every ZIP member is checked against its manifest, and internal conversion history remains excluded. Workstation raw captures remain unchanged; transparent background packing/resume is still separate work.
- [x] Add one disk-streaming background post-capture packer with coalesced wakeups and durable tasks; enqueue complete verified captures only, never host-written partials. New scans use managed packed retention; old journals retain raw storage until explicitly changed. Worker chatter does not obscure swap cues. True host-stream compression remains optional future work.
- [x] Bind original/packed bytes, SHA-256, codec/version; independently decompress before atomic container/sidecar publication and reverify the published pair before raw retirement under exclusive ownership. `storage pack N` retains raw by default; `--retire-raw` is explicit. Acquisition metadata remains unchanged and original SCP bytes are recoverable from the container.
- [x] Support packed captures transparently in decode, flux status/hash checks, completed recovery resume, storage benchmarks and archival export. Use isolated verified temporary SCPs with shared-reader/exclusive-packer locks. Packages retain verified containers/bindings rather than excluding all ZIP evidence; logical original hashes remain stable.
- [x] Persist queue state and test interruption, corrupt/truncated compressed files, disk-full, failed verification, repeated resume, cleanup and cross-process contention without evidence loss or duplicate ownership.
  - [x] Durable tasks and `storage resume` work offline; failed tasks/raw evidence remain, duplicate owners are refused. Tests cover ZIP-before-sidecar restart, repeated packing, corrupt container/member/hash refusal, reader/packer contention, isolated materializations/normal cleanup and recovery reuse. Windows preflights capacity. Real isolated copy: all 26 captures 1,032,982,606 -> 243,529,973 bytes in 27.07 s; 20/20 disks hash-healthy, packed 007 decoded identically, original pilot retains 26 raw files.
  - [x] Add forced-process termination at 11 packing checkpoints and three materialization checkpoints, injected mid-write disk-full errors for ZIP/binding/materialization, capture-bound orphan cleanup and eight cross-process two-reader/packer contention rounds. Durable tasks resume; corrupted evidence blocks abandoned-copy cleanup, active readers are skipped, unknown/edited/foreign/legacy scratch is preserved and a killed queue owner's lock releases. A separate saved 007 copy roundtrips through release packing/status/benchmark with unchanged source hash. Process-crash/injected-I/O coverage, not power-loss certification. See `CAPTURE_STORAGE.md`.
- [x] Add shared controller-process admission for packing, recovery/audit and conversion: acquisition-first CPU headroom, sampled RAM/volume-space estimates, same-volume reservations and one background bulk-I/O slot; apply bounded observable backpressure before taking publication/storage locks. Existing tools finish normally. This is admission control, not hard OS quotas or proven physical throughput; retain the full contention/136-disk acceptance gates. See `BACKGROUND_PROCESSING.md`.
- [x] Document forensic raw/packed retention and archival export. Containers preserve exact physical flux, not regenerated or sector-only substitutes; archives retain original hash bindings. Raw retirement follows the managed policy or explicit offline flag and is reversible; the original customer pilot/archive remains unchanged.

## 11. Autonomous two-drive production workflow

Greaseweazle-only production is also a first-class mode; no USB scan is required. Build its guided disk-swap loop first around the single-disk recovery service, then add concurrent USB/GW scheduling. The working Mitsumi stays connected; alternate-drive comparison is an optional service action, not routine operator work.

- [x] Add a guided Greaseweazle-only batch loop with automatic numbering, custody confirmation, concise swap cues, resume, and background downstream work. Existing scan/live gates now join the production endpoint/archival tail; numbered or explicit Enter-only confirmation stays required. Full 136-disk physical acceptance remains open below.
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

### Deferred alternating GW A/B preload mode (operator request, 2026-10-09)

Deferred by the operator until a suitable two-headed A/B cable is available. Current straight or single-headed twisted cables cannot preload two stations. Keep the concept below; it is not the next implementation priority. A second USB drive may become available separately, but no distinct second USB station has been confirmed/tested.

- [ ] Add an opt-in two-drive/single-board alternating scan mode to reduce swap gaps and overlap saved-flux decoding with the next physical capture. Provisional spelling: `fv scan --gw-double` (not implemented; final naming to be decided). Ordinary single-drive scans remain selector B; `--double` retains its existing USB + GW meaning. This is serial board access with pipelined CPU work, not simultaneous A/B capture.

  Requested workflow and implementation/acceptance requirements (one feature, not separate completion claims):

  - Preload 001 in selector A and 002 in selector B; this mode starts on A. Keep each drive's physical disk identity/custody independent and allocate subsequent fresh labels exactly once.
  - As soon as A's physical capture has completed, its source handles/board operation have released, and raw evidence is safely preserved, decode A offline while starting B's next eligible capture. Serialize **all board-accessing commands** through one board-wide owner; offline conversion must not secretly probe the board. Never launch two physical readers on the same board.
  - If the other drive already contains the next eligible preloaded disk, automatically start it when the board becomes available under the selected confirmation mode. If empty, display an INSERT NEXT cue and accept Enter (`--no-verify`) or its exact number (verified mode). Missing media must become an operator action, not a crash, silent numbering advance or unbounded probe/retry loop. Validate whether bounded read-only presence/index checks can reliably distinguish absence from an unreadable/no-index disk; never treat presence as proof of its label.
  - After decoding determines that a disk needs no further physical recovery, show REMOVE / INSERT NEXT for that station while the other capture continues. Alternate A/B thereafter. A partial/raw-only result still needs verified preservation before a removal cue; never release a disk while a targeted reread or recovery decision is outstanding.
  - A decoded problem disk stays in its station and queues automatic targeted recovery against the same disk/selector. The other drive may use the board meanwhile, but stop/escalation/priority rules must not starve recovery, lose a stage, or tell the operator to swap a disk that will need another pass. Charge stage budgets only for their own capture work, not offline decoding or waiting for the other station.
  - Use a non-blocking input/event pump like existing `uN` / `gN`: station-addressed confirmations (e.g. `aN` / `bN`), status, QUIT and STOP must not suspend an active read. Enter-only confirmation must identify one unambiguous prompted station. Define explicit preload/arming semantics before implementation, especially for automatic starts in verified mode: merely sensing a disk cannot certify that it is the next fresh label or that a prior SAVED disk was actually replaced. No accidental automatic reread of the disk still sitting in the other drive.
  - Persist per-station loaded/armed/reading/recovery/saved/removal state, global fresh numbering, queued captures/decodes, and immutable evidence bindings. Resume requires pending-label reconfirmation, respects completed evidence, and never duplicates an image or advances past an uncommitted label. Retain existing STOP/Ctrl+C/generation-bound stop, graceful QUIT drain, endpoint/no-next-insertion cues and background processing/packing.
  - Keep source operations strictly read-only and require protected media. Qualify the A/B cable, power supply and both drives first; the potentially misaligned Samsung is not presumed healthy. Test empty/stale-loaded drives, wrong labels, decode-triggered recovery, two simultaneous swap prompts, interrupted capture/decode, cancellation/restart and end-of-range behavior with mocks before physical acceptance.
  - Benchmark against the ordinary B-only scan on comparable disks: separately measure capture time, offline decode time, swap/board-idle gaps, recovery yield and whole-session time. Overlap/seconds saved are a hypothesis until measured; do not claim a second board or doubled throughput. Retain backward-compatible single-GW/USB+GW journals and avoid changing the current 136-disk benchmark configuration mid-run.

### Existing USB + GW concurrent mode

The target setup has two different drives working simultaneously on different floppies. **Speed-first revision: both USB and Greaseweazle scan fresh disks.** GW immediately auto-recovers errors; USB saves a partial image/map and asks to set it aside for GW while continuing fresh feeding. Plain `fv scan` stays GW-only; `fv scan --double --write-blocker-verified` is now an opt-in live pilot, and `--usb` is USB-only. One shared console uses `uN` / `gN`; typing `gOLD` selects an earlier available USB partial. Exact label/protection/removal assertions remain mandatory; dual rejects `--no-verify`. Both workers share one coordinator/project owner, not unrelated scans or competing cursors. Small simultaneous physical acceptance and a damaged transfer precede production claims.

- [ ] Create a central CLI job scheduler with independent USB, Greaseweazle, CPU extraction/recovery, conversion, audit, and packaging worker queues. Current concurrent hardware scope remains one USB + one GW drive, shared labels/transfers, backward-compatible journals and measured contention. Preserve board-wide device exclusion: simultaneous A/B capture is not planned; the serial alternating/preload concept above is deferred for cabling. Later consider optional `--trio` (two USB + one GW), distinct USB identities/reservations/receipts and shared GW recovery routing; not shipped; possible second USB hardware is unconfirmed. Do not implement this as competing scans against one project. See `DUAL_SCAN.md`.
  - [x] Implement the backend-independent persistent two-station coordinator: fresh labels on either station, generation-tagged custody, USB partial set-aside/removal/queued-GW transfer, exact old-label selection, verified source receipts/cross-station readable-byte consistency, interrupted reconfirmation, single writer and bounded atomic journal. Synthetic overlapping workers and a reopenable 136-label/14-transfer model pass. `scan --double --plan` / `production status` remain offline-only; the live adapter checkpoint below connects readers/shared downstream ownership. See `DUAL_SCAN.md`.
  - [x] Wire opt-in live dual pilot: one bounded input/event pump, independent USB/GW read workers, station-specific progress/red-green swap cues, fast-pass USB partial routing, earlier-label GW selection, shared-owner background processing/packing, selector/endpoint persistence, graceful drain and source-preserving restart. USB/GW reservations exclude competing readers across projects. Guard transfers before image publication, including completed GW reuse; adopt completed USB evidence after a lost receipt without opening a device. Event-pump and actual mock-subprocess/restart tests pass; per-station timings/results are saved in unique dual-run reports. Physical acceptance remains open.
- [x] Run the USB and Greaseweazle physical drives concurrently on different disks without blocking hashing, extraction, conversion, or reporting workers. The 007–010 live dual pilot completed five station reads with overlapping USB/GW work and five background processing runs; USB 009's three missing sectors were recovered by GW. This small cohort is not full 136-disk throughput acceptance.
- [x] Automatically triage every completed dual-station USB result under the requested speed-first policy: clean acquisition -> USB complete; any missing sectors -> move to GW without extra USB retries. Persist policy/route/attempt/hash/map counts and saved-ticket generation with the sealed receipt, validate on reopen, and retain the original decision after transfer. A source-read failure stays interrupted, not an invented completion; even all-bad saved images remain GW-eligible rather than being called globally unrecoverable. Standalone legacy USB scanning remains a separate workflow.
- [x] Automatically prioritize the GW transfer queue by bounded scheduling-value/cost heuristic, severity, age and custody availability. Removal-confirmed disks outrank still-held USB partials; missing-sector count and boot loss inform score, later-ticket generations give a capped anti-starvation bonus, legacy age stays unknown. Concise GW ACTION cues recommend a disk, exact available labels can override, and `production queue`/JSON explains sealed inputs/reasons without reading media or changing custody. Selection/recommendation snapshots and USB triage enter durable dual telemetry. These are heuristics, not recovery-yield probabilities, file-importance inference or wall-clock wait estimates. Mock CLI/restart/136-label/tamper tests cover policy and guards; live mixed-priority acceptance remains separate.
- [ ] Maintain unambiguous disk identity/custody so results from two drives can never be attached to the wrong floppy number.
- [x] Require simple numbered station/protection/removal confirmation for transfers; verify matching geometry and every mutually readable sector before publishing a new GW image. Reject unknown/mismatched shared bytes and changed USB seals; retain raw evidence/queue identity. This supports consistency, not proof of physical label identity; live transfer acceptance is separate.
- [ ] Keep both drives busy whenever eligible work exists; CPU-heavy extraction/conversion must not stall physical acquisition.
- [x] Allow continued USB feeding during an earlier GW recovery: independent live adapter workers/event pump, tested with a blocked mock GW while USB saves another disk. Confirm this on the small physical pilot before claiming a measured speed gain.
- [ ] Use audible cues and unmistakable terminal messages differentiated by station: **USB swap**, **move to Greaseweazle**, **Greaseweazle swap**, and **attention only if automation is exhausted**.
- [x] Support pause/resume and clean shutdown while preserving every queue item and in-progress artifact safely. Existing supported queue/controller cancellation, graceful drain, persisted jobs/partial artifacts and exact-label resume satisfy this operational contract; broader power-loss boundaries/physical driver behavior remain open separately.
  - [x] Dual feeding PAUSE/RESUME (p/r) persists across restart, blocks claim/confirmation/queued switching and implicit saved-disk removal before a new read, lets existing reads publish and accepts explicit removal while paused. Resume starts no reader; graceful QUIT still drains. Active cooperative STOP/Ctrl+C and generation-bound second-console `fv stop` now cancel supervised hosts/shared workers, preserve pending custody and require reconfirmation; `fv start` aliases scan and `fv run status` checks the owner offline. Actual mock single/dual capture and decode stop/restart, simultaneous mocked USB/GW, Office issue retry, packing retirement and partial-package tests pass. Live WinWord B stop/resume retains stage time and publishes one verified partial image. Blocked USB reads, every power-loss/publication cutpoint and broad physical production acceptance remain separate. See STOP_RESUME.md.
- [ ] Estimate throughput and remaining batch time from observed read/retry/conversion durations.
  - [x] Dual scan shows current-invocation saved-label pace and a bounded rough fresh-feed ETA after three distinct fresh saves with an endpoint. Count in-flight initial reads, deduplicate USB/GW transfers, suppress ETA while paused and keep outstanding recovery/file tail outside the estimate. Saved old transfers cannot supply fresh ETA samples; duration includes swaps/pauses, not just summed reader time. Full recovery/conversion/tail prediction remains open.
- [ ] Add a production acceptance benchmark for the 136-disk reference job: complete ordinary dual-drive acquisition/recovery and downstream processing within a target six-hour operator session.
  - [x] Persist bounded, synced dual-session events before reader launch and after verified receipt publication; record pauses, explicit removals, failures and feed/full completion. Automatically export immutable JSON/receipt CSV and provide offline `production benchmark`. Replay checks project/session/ticket/hash/map seals, deduplicates labels/receipts, measures reader union/overlap and leaves interrupted total durations unknown. Tests cover concurrent mocked transfer, actual CLI kill/reopen, malformed/truncated/tampered records and customer-package exclusion. Saved pilot inspection preserves all 179 source artifacts; mock-only release smoke passes. Older runs retain timing gaps; the physical 136-disk benchmark remains open.
- [ ] Record operator touches per disk and target the theoretical minimum: initial insertion/removal plus one Greaseweazle transfer only for escalated disks.
- [x] Provide an unattended supported-workflow tail: saved-capture decoding is reconciled by scan/recovery, then production automatically finishes extraction/conversion/audit/verified packaging without media. Interrupted finishing rechecks saved evidence and resumes offline. Raw-only unsupported formats archive explicit zero-file-yield attention reports; broader alternate-profile decoder searches remain open.

## 12. “Mini electron microscope the shit out of it” flux recovery diagnostics

- [x] Track/head map for raw-flux capture quality. `fv diagnose N` exports capture availability and bounded per-revolution transition/pulse/index/RPM measurements from verified raw or packed SCP; measurements are not magnetic quality, alignment or CRC certification.
- [ ] Per-track decoded sector summary: present, valid CRC, bad CRC, missing, duplicates/unusual IDs where available.
  - Implemented per-track vendor-reported good/unavailable/unobserved summaries. The saved host map combines bad CRC/missing; per-revolution CRC and duplicate/unusual IDs remain explicitly unknown, so the richer decoder requirement stays open.
- [x] Compare multiple revolutions/passes through text/JSON diagnostics and exportable data. Export raw revolution measurements, profile-separated decode changes/conflicts and replayed final-sector CSV; targeted unobserved tracks never become false losses.
- [ ] Report weak/problematic regions and which decode attempt recovered each sector.
  - Implemented exact missing/conflicting LBA/CHS and supporting capture/decode identities, including distinct raw-hash counts. Physical weak-bit/local timing classification remains open; raw pulse statistics alone are not proof.
- [x] Re-run decode from the same raw flux with alternate supported Greaseweazle profile without touching the physical disk. Existing immutable offline `greaseweazle decode N --capture-attempt N --profile ...` is tested with separate profile diagnostic histories; arbitrary extra decoder-setting search remains outside this command.
- [x] Compare results from USB-sector reads versus Greaseweazle-derived sector images. Saved catalog attempt comparison and dual pre-publication guards compare mutually readable bytes/geometry/maps, reject changed/conflicting/unbound inputs and retain USB queue/evidence on refusal. Live 053–075 transfers and mock conflict/no-shared-sector/tamper tests pass; this is not independent flux CRC certification or unrestricted donor compositing.
- [x] Composite/reconstruction tools must retain provenance and never masquerade reconstructed bytes as an untouched original capture. Existing DERIVED handoff/provenance tests plus independent diagnostic final-sector/image replay, managed-path refusal and changed-provenance refusal enforce this distinction.
- [x] Export a recovery note describing what was physical capture, decoded data, retry-recovered data, and reconstructed/composited data. Saved diagnostics list committed missing/conflicting LBAs, pass changes and evidence boundaries; derived/reconstructed bytes remain separately labelled, never new original observations. See `FLUX_DIAGNOSTICS.md`.

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
- [x] Add initial size/observed-duration-aware scheduling: interleave one estimated-long job with three estimated-short jobs in the shared queue; idle workers take the next job immediately, with exactly-once claims and stable report order. Use successful hash-matching prior durations, otherwise source size. Shared adaptive CPU/RAM/volume-space admission is now implemented; physical 4/8/12 performance acceptance remains open.
- [ ] Benchmark 4/8/12 conversion workers against saved pilot files, measuring throughput, peak memory, timeouts and output integrity; workers are concurrent jobs, not a guarantee of one CPU thread each or linear speedup.
  - [x] Exercise a fresh 172-job saved-image run at 12 workers: 164 OK/eight partial DOCX timeouts under the restricted desktop runner (process-tree termination reported access denied). Retry those eight at four workers outside that restriction restored 172 OK/zero exceptions, reusing 336 outputs. Subsequent canonical-path 12-worker processing reused all outputs with zero new LibreOffice conversions and restored the 17 verified/three attention evidence result. Keep four as the conservative default; do not infer twelve is a validated faster production preset.
- [ ] Stress-test mixed-size scheduling, same-stem/output collisions, concurrent CLI runs, retries, cancellation/restart and worker failure; require exactly-once claims/publication, isolated LibreOffice profiles, hash-bound reuse, serialized shared metadata/audit writes and complete deterministic reports.
  - [x] Test mixed-size scheduling with 12 workers, invalid/duplicate schedule refusal, bounded concurrency, deterministic report order and small-job progress while a large job remains active.
  - [x] Reserve the project conversion lock across delivery planning, all workers and state publication; a second conversion/plan owner is refused. Atomically publish the state cursor and preserve immutable new/prior snapshots in `Reports/ConversionHistory`.
  - [x] Fix DOS versus extended/canonical Windows path identity when finding prior hash bindings and selected retries; changed source/output hashes remain refused. Tests cover canonical-path reuse without starting a tool and malformed state refusal without overwriting the cursor.
  - [x] Resolve both output and Converted root before writing relative conversion-summary paths; reject missing/outside outputs instead of falling back to an absolute path. Canonical/DOS root mismatch regression and escape tests pass. Offline reprocessing of the live dual 007–010 pilot changes evidence audit from three false conversion warnings to four verified disks, reusing all 58 bound outputs without changing saved images/captures/extracted/delivery bytes.
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

- [x] Per-floppy audit state combines acquisition, image quality, extraction, recovered-file count, conversion status, and generated-file integrity.
- [x] Preserve useful statuses such as `OK`, `PARTIAL: IMAGE READ`, `PARTIAL: CONVERSION`, `CHECK: CONVERSION FAILED`, `CHECK: NO RECOVERED FILES`.
- [x] Summary metrics equivalent to the current final report.
- [x] Recovered-file inventory with recovery method + SHA-256.
- [x] Conversion result inventory and conversion-issues subset.
- [x] Generated-file integrity inventory.
- [x] Delivery-file manifest.
- [x] Export CSV/text reports.
- [x] Export `FloppyFinalReport.xlsx` equivalent from the app or a dedicated report exporter. Immutable bundles and a hash-bound complete-generation pointer replace the original final-audit script's useful outputs without Excel COM; not historical-yield certification or identical CSV schemas. See [FINAL_REPORTS.md](FINAL_REPORTS.md).
- [x] Generate polished XLSX reports directly rather than depending on Excel COM automation.
- [x] Primary report language is Hungarian.
- [x] Add English report export from the same underlying report data model. `report export --language en`; Hungarian remains default, stable machine/status names and recorded forensic warnings are retained.
- [x] Excel summary/dashboard sheet with major KPIs and project statistics.
- [x] Include charts for useful project-wide metrics such as imaging status, bad-sector counts, recovery results, file counts, and conversion outcomes.
- [x] Detailed per-floppy worksheet/table with filtering, frozen headers, sensible column widths, status highlighting, and consistent formatting.
- [x] Separate recovered-file, conversion, issue, and integrity tables where useful.
- [x] Reports should be presentable to a customer without requiring manual cleanup in Excel.
- [x] Audit must be re-runnable/idempotent and never alter source floppy media.
- [ ] Audit runs automatically after every material state change and at batch completion; no manual spreadsheet update step remains. Scan/process background and final reconciliation automatically produce the combined report. Direct expert mutations still need process/report refresh; all-command invalidation/refresh remains open.

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
- [x] One **Finalize project** action automatically refreshes recovery/extraction/conversion/audit state, builds the package, verifies it, and reports only unresolved exceptions. `finalize` now keeps one owner/control generation throughout, records bounded durable phases, supports offline `status`/`resume`, and offers explicit `--allow-attention` archival packaging (still exit 3, never customer certification). Default clean-only behavior and fatal integrity refusal stay unchanged. Automatically starting at the scan endpoint remains the separate production-policy item below. See FINALIZATION.md.
  - [x] Add CLI `finalize --destination PATH` for existing images: run the shared processing pipeline, stop packaging when attention remains, and build/verify an archival ZIP only after a clean run. Automatic production policy remains open.
- [x] Optional production policy automatically builds the final package when the last physical disk and all background queues are complete. Choosing `production start` opts into archival partial-results-with-attention policy; journal/receipt hashes, full range and pending USB transfers are checked before finishing. Ordinary `scan` and strict-default `finalize` are unchanged.

## 16. Current dataset regression targets

### First workable pilot — single Greaseweazle station

- [x] Simplify GW custody to number-only input (`004`/`4`) or `QUIT`, retain wrong-number/blank refusal and legacy READ compatibility.
- [x] Add ordinary `scan` for GW with saved per-project defaults and an installer-provided `fv` alias; preserve explicit guarded USB scans and expert overrides.
- [x] Explain built-in/saved recovery budgets as optional expert configuration rather than mandatory operator homework (`POLICIES.md`).
- [x] Reproduce/fix host exit-zero `Command Failed: No Index` reporting; preserve failed metadata and allow an operator-confirmed expired empty-first-capture restart without resetting jobs that contain raw evidence.
  - [x] Keep guided scanning open after a specifically classified No Index capture failure: red same-disk remove/reinsert cue, explicit reconfirmation (number/Enter mode), two bounded reseat retries per disk/invocation, preserved failed metadata/partial flux and normal stop for other errors. Mock tests cover wrong labels, EOF/QUIT, retry cap, verification refusal, next-disk reset and cross-process custody/JSON/telemetry.
  - [x] Validate the reseat prompt on live 022 in the existing 021–032 project: two prompted failures followed by a successful capture/recovery sequence, historical failures retained, verified partial saved with two missing sectors and cursor advanced to 023.
  - [x] Treat a capture timeout caused by the exhausted whole-recovery budget as a normal partial stop when earlier decoded passes verify; preserve interrupted SCP/metadata separately, exclude them from result evidence/packing, and continue numbered feeding. Unexpected operation timeouts, empty first passes and integrity failures still stop. Cover deadline/resume/error cases with mocks and validate saved 023 offline on an isolated copy.
  - [x] Give each recovery stage an independent physical-capture budget (default ten minutes), remove the hidden five-minute host-read ceiling, preserve spent time across failed/reseat attempts and process restart, migrate unfinished legacy jobs without rereading completed passes, and continue escalation from verified evidence after a later stage exhausts its allowance. Mock/clock/slot tests and a saved-023 intercepted-read regression cover the change.
  - [ ] Finish the remaining live 023–032 cohort in the same project, retrying the pending 023 Detective stage with its independent allowance and collecting extraction/benchmark results; verify live stage-budget exhaustion/escalation without exiting.
  - [x] Complete and collect actual live 021–032 continuation: 12 project images / eight OK / four partial, cursor 033, completed capture packing, 37 successful conversions and zero downstream errors. Pending 023 completed Detective and stopped at its pass limit; historical five recovery errors remain preserved. Actual live ten-minute stage exhaustion/escalation was not triggered and remains the separate acceptance condition above.

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
  - [x] Fresh 009 automatic-DD smoke test (2026-10-07): 1,440 readable sectors, one capture/109.8 s, correct automatic format, background drain/packed retention and verified JPEG; image/payload bytes identical to the earlier physical pilot.
  - [x] Fresh 058 repeat (2026-10-07): 2,880 readable sectors, one capture/105.1 s, five successful document conversions; image and all extracted payload hashes match the earlier 053–064 scan. Historical archive differences reproduce, not acquisition drift. Operator confirms read animation/clearing works.
  - [ ] Compare live 009 against the archive's signature-carved outputs (including a large legacy Word candidate and JPEG), distinguishing deleted/orphaned content, recovery-tool reports and actual reachable files. A clean FAT extraction is not equivalence to legacy carving, and whole-image/payload hashes currently differ.
- [ ] Run the first 136-disk physical pilot and collect benchmark/audit/recovery artifacts, preserving the original script archive for comparison.
- [ ] Compare pilot source/recovered-file hashes and yield against the script/DMDE baseline, then prioritize changes using measured failure/throughput data.

Use the supplied `FloppyFinalReport.xlsx` and existing archive as regression truth while porting functionality.

- [x] Import/represent all 136 floppy records. Original script ZIP passes an isolated CLI import with 136 discovered legacy attempts and next cursor 137; all 6,261 original members/683,329,461 bytes independently match the ZIP, source hash unchanged. Per-disk historical status/hash/extent/folder counts are reported; old aggregate reconciliation and automated recovery yield remain separate.
- [ ] Current reference summary: previously documented 136 images present; 94 imaging OK; 42 imaging not OK; 86 floppies fully OK; 50 need attention/are partial. The 2026-10-10 exact archive import discovers 136 images; current primary-log parsing reports 102 OK / 28 PARTIAL / 6 MISSING LOG, while the archived index has 126 rows / 88 OK / 38 PARTIAL. There are 38 historical discrepancies (three log hashes, 34 index hashes, one log extent), not failed copied-member checks. Reconcile original workbook/index/log aggregates before asserting equal classifications or full-file yield; original claims are preserved, not overwritten.
- [ ] Reproduce 1,667 recovered source-file records and the current conversion/audit counts when pointed at the same archive contents.
- [x] Correctly represent severe cases rather than assuming every image is 1.44 MB. Actual original-ZIP CLI import independently verifies every member, preserves 009 at 737,280 bytes and 133 at 417,792 bytes, and flags the latter's inconsistent reported log extent without padding or inferred recovery. No new intact-file/yield certificate follows.
- [ ] Regression-test examples with 1 bad sector, tens of bad sectors, hundreds of bad sectors, conversion-only failures, no-recovered-file cases, manual recovery, and signature recovery.
- [ ] Measure automated recovery yield against the existing manual DMDE/script results; FluxVault must match or exceed recovered verified files wherever the same evidence is available.
- [ ] Track operator interventions required for all 136 disks and drive the normal technical-decision count toward zero.
- [x] Benchmark a simulated/fixture-based two-drive run before using customer media, including queue scheduling and crash-resume behavior. Existing modeled 136-label/14-transfer scheduling/reopen soak, simultaneous USB/GW worker fixture, real mock-CLI receipt/crash/stop/pause tests and generation-bound telemetry replay satisfy the fixture gate. Physical full-136 throughput/yield acceptance remains open.

## 17. Testing

- [x] Unit tests for floppy-number parsing and zero-padding.
- [x] Unit tests for legacy archiver-log parsing.
- [x] Unit tests for DMDE multi-pass map replay (later successful `C` replaces earlier `E`).
- [x] Unit tests for path cleanup / delivery naming / collision handling.
- [x] Unit tests for Greaseweazle command construction, especially raw-flux safety flags.
- [x] Unit tests for project persistence and migrations. Schema-1 legacy metadata/cursor compatibility, read-only opening, atomic save failure, preserved extension/tool/operator fields, malformed/oversized/future-schema refusal and full legacy script-ZIP migration are tested. No automatic migration of an unsupported future schema is claimed.
- [x] Unit tests for SHA/integrity helpers.
- [x] Fixture-based tests using scrubbed/sample logs and tiny synthetic images; never require a customer floppy for automated tests.
- [x] Native FAT12 fixtures cover fragmented chains, directory gaps, FAT-copy fallback/conflicts, FAT entries crossing sector boundaries, loops/cross-links, unsafe paths and 701-bad-sector recovery of an independently intact file.
- [x] Native generation-5 regressions cover lost-directory ownership/cycles/gaps, ambiguous/invalid/bounded missing-link suffix hypotheses, binary Word version bindings/mini-streams/Unicode/text holes, exact source replay, CLI/delivery origin labels, archive inclusion without whole-file inflation, tampered-output refusal and native-v4/fragment-v1/deleted-v1 compatibility. Saved-image checks use isolated copies and preserve original acquisition bytes.
- [x] Word-text-v2 regressions cover sparse CFB v3/v4, fragmented regular/mini streams, both FIB table selectors, required versus unrelated directory holes, detached/duplicate root identities, allocation cycles/crosslinks, bounded malformed input, HTML escaping/immutability, old-v1 preservation and archive/count separation. Isolated 001–020 / 021–032 source replay verifies all decoded extents, allocation links, CP coverage and regenerated HTML; all previous 64 payloads and 39 Office conversions remain compatible.
- [x] Native service/CLI tests cover hash/map refusal, immutable source/manual preservation, inventory/report tampering, verified reuse and partial files flowing through batch extraction, manifests, delivery mirroring and audit with no hardware.
- [x] Preserve valid DOS installer underscores and escape-prefix uniqueness; select managed generations numerically so legacy native output cannot hide a later numbered extraction.
- [x] VFAT name fixtures cover Unicode/surrogate pairs, exact 13-unit boundaries, padding/type/checksum errors, directory gaps/deleted entries, fragmented-directory slots, unsafe names, alias/Unicode collisions and ASCII short-name case flags.
- [x] Native generation upgrade test preserves first-generation files/reports/schema compatibility; OS metadata remains forensic but is omitted from new delivery mirrors and verified packages.
- [x] Validate native recovery on the saved WinWord 1 Greaseweazle image: 22 intact files, unchanged image hash, zero new physical reads, repeat reuse and downstream partial audit/workbook.
- [x] Integration test for 7-Zip adapter.
- [x] Integration test for LibreOffice adapter when installed.
- [x] Greaseweazle hardware tests marked/isolated so normal `cargo test` works without hardware.
- [x] End-to-end automated fixture test: numbered mock acquisition -> automatic extraction -> real LibreOffice DOCX/PDF -> audit -> `finalize` verified archive, with original bytes, DOCX text, PDF header, ZIP members/CRC and archive SHA-256 checked. Invalid-filesystem attention blocks finalization instead of creating an empty-success package. Uses disposable FAT12 and installed 7-Zip/Office; environment-dependent and explicitly run. Damaged native recovery has separate fixture/cohort checks, not whole-collection yield certification.
- [x] Scheduler tests prove USB/GW worker overlap and earlier-label transfer with sealed artifact receipts; repeated completed, skipped and out-of-scope labels refuse without releasing custody/changing the journal or starting a read. Actual 053–075 cohort retains 23 identities/29 receipts; offline conversion-transfer repair verifies all 1,333 existing acquisition/extraction/delivery artifacts unchanged. This does not prove a mistyped physical label or full multi-station production throughput.
- [x] Policy tests cover automatic escalation, bounded retries, no-improvement stopping, severe-damage carving, and unrecoverable outcomes. Runner mocks plus automatic native-fallback/400-bad-sector/zero-candidate fixtures cover the implemented supported-format path; these are not hardware yield certification.
  - [x] Mock tests cover clean fast-pass stop, targeted escalation, no-improvement stop, recovered-sector provenance, control-byte conflict refusal, absent-board refusal, policy-limit validation, output tamper refusal, and offline decode resume after expiry with no board.
- [x] Long-run soak test models 136 disks, application restart, worker failure, and resumability. Scan fixture now closes/reopens the project between sessions; a separate 136-job cohort fails downstream, reopens, drains exactly once, deduplicates repeated enqueue, preserves source hashes and does not loop attention results. This is deterministic modeled coverage, not a full live run or a forced-process storage soak.
- [x] Clear legacy Clippy warnings and enforce strict all-target linting. Compiler-suggested mechanical fixes, smaller boxed channel events, named callback/artifact types and compile-time write-safety assertions pass `cargo clippy --all-targets -- -D warnings`. Narrow documented orchestration-arity exceptions and the intentionally orphaning test host remain; no blanket lint suppression. Full regressions verify unchanged recovery/cancellation behavior.
  - [ ] Investigate one intermittent Windows concurrent test-harness fast-fail (`0xc0000409`) observed during storage development; isolated storage tests and subsequent complete reruns passed. Do not treat the unreproduced event as a diagnosed/fixed defect; keep it in soak-test acceptance.

## 18. CLI / automation interface

All CLI commands must call the same guarded Rust workflow services so safety, provenance, validation, and output formats cannot drift.

- [x] Install a `fluxvault` executable that can be added to `PATH` and run from PowerShell, CMD, or another automation process. Current release installed into an isolated spaced temporary folder passes PowerShell/CMD PATH resolution, `init`/JSON status and direct automation; aliases match release hashes. WhatIf creates nothing, user/process PATH stays unchanged without opt-in. Actual user installation remains an operator choice; this test changes only child-process PATH.
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
- [x] Human-readable output by default plus stable `--json` output for scripts; progress goes to stderr so JSON/stdout remains machine-readable. Existing executable scan/recovery/import/report/production/error/stop contracts plus new dashboard/lifecycle tests verify supported commands. New versioned objects are additive; historical/unknown states, exit codes and redirected/color/sound behavior remain explicit.
- [x] Stable documented exit codes for success, partial recovery, operator action required, invalid project/input, missing tool, and fatal failure. STOP_RESUME.md documents their shared-category mapping and operator cancellation; subprocess tests verify the contract. Distinct codes for each kind of error are not implied.
  - [x] CLI contract: 0 complete within command scope, 3 attention/partial/operator decision, 2 invalid input/project/missing tool/operation failure, 130 operator cancellation. `audit` and `process` return 3 for recovery/conversion attention. JSON cancellation errors use operation_cancelled; other errors retain operation_error. Help options cannot create an init project or start tools/media.
- [ ] Non-interactive/background operations require explicit policy flags; physical-media operations retain read-only safety while confirmations are limited to unavoidable custody/media changes.
- [x] Every CLI external-tool invocation uses argument arrays and the shared command/audit log; never expose Greaseweazle write/erase commands.
- [x] Generate optional PowerShell 7+ native Tab completion: `fv completions powershell`, bundled installer script, command/flag/fixed-value suggestions with USB/dual filtering, literal AST input and no tool/media/project access or automatic profile edits. Actual engine/cursor/full-path tests pass. Bash/Zsh remain deferred until cross-platform support. See SHELL_COMPLETION.md.
- [x] CLI integration tests cover project discovery, JSON schemas, exit codes, resumability, and safe failure without physical hardware. These test implemented command contracts, not all future commands or every power-loss boundary.
  - [x] Exercise the built executable against a disposable nested project: project discovery, JSON output/errors, exact exit codes, and a guided-scan quit path that never enumerates or reads a drive.
  - [x] Cross-process LibreOffice integration test proves hash-bound reuse, tampered-output refusal, persisted issue loading, and retry after restart using only a disposable RTF.
  - [x] Cross-process GW recovery test seeds saved raw evidence, publishes a compatible image, then reuses the completed result with the mock board absent and no new read commands.
  - [x] Add interrupted-acquisition resume integration scenarios without requiring physical media: forced termination and cooperative typed/separate-console STOP during actual single/dual CLI mock captures and single decode; retained Windows handles confirm host/descendant exit, partial evidence and pending labels survive, one image/custody identity resumes and completed flux decodes offline without another raw read. Prompt stop needs no EOF; stale requests cannot stop new generations. Mock Office cancellation retains isolated partial output and resumes saved issues. Unrelated processes remain alive. Broader cutpoints/physical acceptance remain separate.
- [x] `fluxvault production start` runs the shared scheduler and prints concise swap instructions while supported technical decisions remain automatic. Single-GW default; `--double --write-blocker-verified` reuses USB + GW and exact uN/gN custody. `production resume` restores options; one owner/stop generation spans feeding through verified ZIP. Mock dual end-to-end and USB-transfer endpoint guards pass; broader live acceptance remains open.
- [x] `fluxvault audit` and `fluxvault package build --destination PATH` use the evidence-audit and verified-package services without application-wide project state.
- [x] `fluxvault process` runs the existing-image extraction -> Office conversion -> evidence audit -> workbook pipeline with no physical drive access; progress goes to stderr and `--json` output to stdout.

## 19. Milestones

- [x] **M0 — CLI foundation:** module layout, settings, project create/open, guarded workflow services, and folder-first commands. The earlier desktop shell was retired.
- [x] **M1 — WORKING USB ARCHIVER:** safely image a real floppy, retry/fallback, bad-sector map, SHA-256, persistent project record.
- [x] **M2 — SCRIPT REPLACEMENT CORE:** import legacy archives/logs, auto extraction, recovery queue, manifests. Core services now shipped and tested, including actual original 136-image ZIP import. Historical yield/schema equivalence remains a separate open acceptance target, not implied by this milestone.
- [x] **M3 — AUTOMATED RECOVERY CORE:** multiple USB attempts, policy-driven compare/composite, FAT12 reconstruction, carving, provenance, and legacy DMDE import compatibility. Existing bounded supported-format engine/fixtures/live saved cohorts satisfy the core milestone; broader damaged-tree/container reconstruction and full DMDE yield parity remain open in sections 8/16.
- [x] **M4 — GREASEWEAZLE READY WITHOUT HARDWARE:** tool detection, mocked backend, safe command construction, raw/derived artifact model. Mock runner/recovery/decode/timeout/stop/safety and immutable evidence contracts pass without a connected board; milestone reconciled with already shipped work.
- [x] **M5 — GREASEWEAZLE LIVE:** raw-flux capture + decode/redecode + detailed flux diagnostics after hardware arrives. Protected Mitsumi/B HD/DD/damaged/stop-resume pilots and saved raw/packed diagnostics validate implemented capture/decode/measurement scope. Weak-bit/alignment/per-revolution CRC classification and full-136 acceptance remain open; no new physical read this checkpoint.
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
