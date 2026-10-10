# FluxVault CLI — advanced reference

Optional PowerShell 7 command/flag completion: `fluxvault completions powershell` prints the static trusted script. [Enable it in one line](SHELL_COMPLETION.md); no project or media access, no automatic profile changes.

For your first run, use [the beginner tutorial](TUTORIAL.md). For daily commands, use [the cheat sheet](CHEATSHEET.md). This reference explains optional controls and evidence semantics; it is not a sequence to paste and run in full.

- [Everyday workflow](#everyday-short-workflow)
- [Saved conversion state](#saved-conversion-state)
- [Capture packing and format discovery](#capture-packing-and-format-discovery)
- [Read failures and restart](#read-failures-and-restart)
- [Installation and command catalog](#installation-and-command-catalog)
- [USB acquisition: optional advanced path](#usb-acquisition-optional-advanced-path)
- [Tools and expert Greaseweazle operations](#tools-and-expert-greaseweazle-operations)
- [Native saved-file recovery](#native-file-recovery-from-damaged-saved-images)
- [Saved-flux diagnostics](#saved-flux-diagnostics)

Examples use PowerShell. `fv` and `fluxvault` are identical aliases. Physical acquisition examples need an identified protected floppy; saved-image processing never needs one inserted.

## Everyday short workflow

For a **fresh full-chain batch**, use `fv production start --last-disk N`. One owned/stoppable workflow joins the scanner to finishing and verified ZIP creation at the replay-checked endpoint. `production resume` restores saved options; output defaults to sibling `<project-folder>-Delivery`. Production explicitly archives partial/raw-only results with attention reports, never customer-certification; early quit/pending transfers do not package. `production status` adds a bounded historical workflow receipt to existing coordinator fields. Single-GW is default; `--double --write-blocker-verified` selects USB + GW. Label shortcut/sound/color/blocker assertions remain per invocation. [Commands and limits](PRODUCTION_WORKFLOW.md).

The installer provides both `fluxvault.exe` and the identical short alias `fv.exe`. `fv init`, `fv status`, and `fv scan` work from the project folder. Plain `scan` uses Greaseweazle; explicit `scan --drive A:` (or USB protection/retry flags) retains the existing guarded USB workflow.

## See what is happening without interrupting it

In another PowerShell window, enter the same project folder and run:

```powershell
fv status
fv disk show 59 --details
fv disk note 59 "Sleeve creased; original label checked"
```

`status` combines recorded station custody, USB-to-GW transfers/recommendation, controller/production phase and processing backlog. Historical feeding pace supplies a rough fresh-feed ETA after at least three samples and a saved endpoint; transfers, recovery/processing/packaging tail and paused feeding do not get a fabricated whole-job ETA. Missing timings remain unavailable. It opens no device or tool and creates no control records. A recorded READING/PENDING state is not proof of an active reader: the original feeding console's current ACTION/swap banner remains authoritative. Conflicting inactive single/dual histories are displayed without guessing a mode.

`disk show` retains the original attempt list and adds a stable zero-padded lifecycle view: automatically ranked image, bad LBAs, extraction state, saved recovery plan/decisions, processing jobs, conversion snapshot and audit association. Historical reports are explicitly labelled; an old verified report does not freshly verify today's output bytes. `--json` adds versioned `operations`/`lifecycle` fields without removing existing fields; stdout stays JSON-only. Inspection may report invalid/foreign controls rather than silently ignoring them.

`disk note N "TEXT"` is optional, not a scan prompt. It does not change disk identity, cursor, preferred acquisition or media. It refuses a busy processing owner, bounds text to 4,096 UTF-8 bytes without control characters, atomically commits under `Reports/DiskNotes`, and retains earlier notes. An empty quoted string clears the current note. A stopped write retains partial evidence. Explicit operator acquisition preference is not implemented: the displayed preference remains automatic evidence ranking.

## Import an original script archive

`fv project import --source ZIP --destination NEW_FOLDER [--plan] [--json]` creates a fresh project from saved script images/logs/recovered folders/reports; no re-imaging, external tools or automatic processing. Parent must exist, destination must not; do not initialize it first or nest it in an existing project. Preview writes nothing and does not certify member payloads. Apply CRC-checks/hashes every member and independently verifies copied bytes before publication. Original hashes/index/logs and short images are preserved; legacy claims/files remain attention, not newly managed recovery or Office certification. Normal status/disk inspection works after import. Exit 0 preview / 3 imported legacy attention / 2 error / 130 stop. Partial staging is retained but not resumable/mergeable yet. [Beginner commands, bounds and measured 136-image validation](LEGACY_IMPORT.md).

## Saved-flux diagnostics

`fv diagnose N` (also `fv greaseweazle diagnose N`) exports saved raw/packed SCP track/revolution measurements, per-track vendor-reported sector counts, pass improvements/losses/conflicts and independently replayed committed sector provenance. The human response prints recovery-note, CSV and JSON paths under `Reports/FluxDiagnostics`; `--json` returns paths/hash/counts/attention. No device or host tool is invoked, and no recovery/delivery choice changes. Code `3` indicates partial/standalone/diagnostic attention; committed hash/provenance mismatches are errors. Physical flags are rejected. [Interpretation, bounds and saved validation](FLUX_DIAGNOSTICS.md).

## Everyday scan controls

`fv start` aliases `fv scan`, with identical options/resume. Scan `STOP` cancels; `QUIT` drains. Windows Ctrl+C requests cooperative cancellation; second-console `fv stop [--project PATH]` requests it too. `fv run status [--project PATH] [--json]` probes the owner offline. Wait for **STOPPED** and drive activity to cease before moving disks. Cancellation exits `130`. [Command scopes, JSON/exit contract and restart](STOP_RESUME.md).

GW scans reuse saved defaults for omitted profile/map, selector, policy, endpoint, storage/processing mode and conversion workers. Explicit overrides remain subject to pending-job consistency checks. Fresh projects default to selector B, automatic IBM 720 KB/1.44 MB discovery, verified packed retention, background saved-file processing, the built-in policy, four workers and no endpoint. Numbered input asserts label/protection checks; individual capture/recover still require `--source-write-protected`. `004` or `4` confirms 004; `QUIT`/`Q` stops. Blank/wrong-number input refuses reads by default. Legacy `READ 004` is accepted. See [POLICIES.md](POLICIES.md).

```powershell
fv scan --conversion-workers 12
fv scan --no-verify
fv scan --color never
fv scan --sound on
fv storage benchmark 7
```

`--no-verify` is an explicit **label-typing shortcut**, not a verification bypass: at every swap, check the displayed number against the physical label and the open write-protect hole, then press Enter. A prominent warning appears at every prompt; hashes, saved evidence, read-only access and restart guards remain enabled. Wrong typed numbers still refuse the read, EOF is never a confirmation, and `QUIT` still stops. The shortcut is **not saved**; supply it on every invocation where wanted. It applies only to Greaseweazle scanning, not USB or offline processing.

Large ASCII banners distinguish **WAITING FOR YOU**, **READING / DO NOT REMOVE**, **DONE / REMOVE / INSERT**, **PARTIAL SAVED** and final processing/results. Clean swaps are green; partial-saved swaps are red with an explicit safe-to-swap instruction. Warnings/raw-only exceptions remain amber, and read failures are red without advancing the number. The endpoint says **BATCH FINISHED**, not insert the next cursor. Automatic color decorates stderr only when it is a terminal, unless `NO_COLOR` is set or `TERM=dumb`; `--color always`/`never` override detection. JSON/stdout remains undecorated even with forced stderr color.

Interactive recovery/scan displays a temporary ASCII bar with disk identity, phase, elapsed time and unique reported tracks out of the host's selected range. Targeted rereads reset that range; duplicate track reports do not inflate completion. Unknown/unsupported ranges and quiet checking/decoding/verifying stages use an animated indeterminate marker. Track visitation is not readable-sector yield. The display is cleared before stage diagnostics and on recovery success/error, before any swap cue. Redirected stderr and `TERM=dumb` do not animate, even with forced color; monochrome terminals retain the ASCII bar.

### Optional scan sound cues

`scan --sound on` (also `start`, explicit GW scan, USB-only scan and `--double`) enables short Windows tones after saved-result/error cues. Default `--sound off` is unchanged silence. This setting is **per invocation, not persisted**; it changes no read, retry, recovery or custody policy. Saved clean results use one tone, partial/raw-only saved results two descending tones, and failed reads/reseat instructions three tones ending low. In dual mode a saved USB partial instead uses three rising tones: **set aside/move this label to GW**. GW partials retain the descending pattern. USB starts at a lower pitch than GW so station cues differ. Cancellation and recurring progress/status messages do not trigger completion sounds.

Sound is supplemental: read the station/label/banner before moving a disk; a sound never authorizes removal. Only interactive stderr on Windows enables audio, even with `--sound on`; redirected output and `TERM=dumb` stay silent. No BEL/ANSI sound bytes enter stdout, JSON or logs, and `NO_COLOR` does not disable requested audio. Native Windows audio respects the machine's audio configuration. One bounded non-blocking worker drops busy/stale notices and discards queued cues at shutdown; audio unavailability/failure never interrupts acquisition. A final cue may be skipped during immediate exit. There is no external player, profile/registry change or physical-drive access for audio.

`scan --conversion-workers N` accepts 1–16 and persists the request at custody commit. Background conversion caps it to available logical CPUs minus two (minimum one); status records requested/effective counts. Isolated LibreOffice profiles and a shared balanced queue interleave estimated long/short jobs, retaining stable report order. Four is the conservative default; more is not guaranteed faster. Whole-project ownership prevents competing report/delivery writers, while short snapshots permit acquisition publication outside Office work. See [BACKGROUND_PROCESSING.md](BACKGROUND_PROCESSING.md).

## Saved conversion state

`process` and scan/background processing now publish verified composite/mirrored-FAT improvements as **DERIVED** image-catalog attempts before extraction. Native recovery, manifests, conversion and audit use that improved evidence automatically; originals/operator folders remain preserved. Ready/reused/declined handoff counts appear in human/JSON results. Even zero-gap derived results remain attention, and changed lineage is refused. Expert standalone `recovery composite`/`recovery fat` commands do not perform this catalog handoff themselves. [Bounds, provenance and restart](OFFLINE_RECOVERY_HANDOFF.md).

Saved conversion state is atomically replaced, with immutable current/prior snapshots retained in `Reports/ConversionHistory` for audit. Equivalent DOS/canonical Windows paths no longer invalidate source/output hash bindings. Changed outputs, changed sources and malformed snapshots are still refused; history is internal, not customer delivery content.

## Capture packing and format discovery

`storage benchmark N` uses saved raw or transparently materialized packed evidence: select the largest verified capture, measure Deflate levels 1/6 and verify byte-identical decompression. Samples are capped at 128 MiB; experimental compressed bytes are discarded. It does **not** pack or retire evidence. Saved 007 measured 77.16% smaller at level 6.

New scans default to `--profile auto --capture-storage packed`. Coherent readable BPB geometry/sector maps select supported IBM formats; nearly complete HD skips the second decode, otherwise both are tried. Without usable geometry, require >=80% readable coverage versus <=5% in the alternative. If both trials complete but neither is convincing, save `raw_format_exception`: verify raw/decision/candidate bindings, stop further physical reads for that disk, and advance custody with an amber raw-only cue. No compatible image/geometry is invented; unknown sector counts are JSON null in telemetry. Tool/decode failures or changed evidence still stop numbering. `identify` saves reports in `Flux/Formats`, with 0 selected / 3 unresolved. Fixed profiles/maps win; old journals retain saved modes. This is bounded IBM discovery, not universal format support or CRC certification.

Packing uses one background worker with streaming 64 KiB buffers, a 512 MiB capture ceiling, ownership locks, capture-bound scratch and Windows free-space checks. Only complete verified captures enter the durable queue. `.scp.zip` contains one exact original SCP; `.scp.packed.json` binds raw/packed sizes, hashes and codec/version. Verify both before retiring raw under the managed policy. Acquisition/decode metadata and logical original hashes remain unchanged. Decode, flux status, recovery resume and archival export handle packed containers transparently. The worker does not obscure swap cues; failed tasks retain evidence and queue state. `storage resume` drains these offline and safely checks abandoned compression/decode scratch, even without a remaining packing task. Real process-kill, injected disk-full and cross-process contention tests pass; adaptive CPU/I/O budgeting remains open. [Cleanup rules and validation](CAPTURE_STORAGE.md).

For existing evidence, `storage pack N [--capture-attempt N]` **keeps raw**; `--retire-raw` explicitly reclaims space after verified publication. `--capture-storage raw` opts scans out. Retirement is reversible by extracting the single SCP member; archival packages include/verify containers and binding metadata. A copy of the real pilot packed all 26 captures 1,032,982,606 -> 243,529,973 bytes (76.42% smaller) in 27.07 s, with 20/20 disks hash-healthy. Original pilot captures remain unchanged.

## Read failures and restart

The runner treats explicit `Command Failed:`/`ERROR:`/`Fatal Error:` output as failure even if the host exits zero. No Index retains failed evidence and gives same-disk reseating guidance. Default recovery uses independently persisted stage clocks: failed/reseat attempts consume their stage allowance, but earlier stages cannot consume Detective's allowance. The historical expert `whole_job` policy retains a guarded empty-first-capture restart path. Completed disks/cursor and incomplete evidence remain preserved. See [policy scope and migration](POLICIES.md).

Guided `scan` handles specifically classified **No Index capture failures** without immediately exiting: a red **REMOVE AND REINSERT SAME DISK** cue appears after the host process stops. Check the same label/open protection hole, seating, power and door/lever, then reconfirm its number (or Enter with `--no-verify`). Wrong numbers, EOF and `QUIT` never start a retry. Up to two confirmed reseat retries are offered per disk per invocation; persistent failure stops with unchanged custody. Retry evidence gets new attempt slots, and failed metadata/partial SCPs remain intact. Timeout, mixed host errors, decoder/tool/integrity errors still stop normally. Reseating cannot be sensed electronically here; confirmation is the operator's assertion. Recovery budgets and read-only protections remain enforced. Telemetry retains the failed operations even after a successful retry.

For a copy-and-paste operator guide, start with [CHEATSHEET.md](CHEATSHEET.md). For the next two short hardware checks and automatic result collection, use [TOMORROW_TEST.md](TOMORROW_TEST.md).

Offline reference comparison: `fv benchmark compare --baseline ZIP [--include-deleted] [--project PATH] [--json]`. Compares original recovered payloads within acquired disk numbers, preserves immutable snapshots and source-image context, and refuses changed managed inventories/unsafe ZIPs. Confirmed deleted reference files are out of scope by default; uncertain/carved content remains in scope. On this command, `--include-deleted` changes comparison scope only. Separate forensic recovery uses `recovery extract N --include-deleted`. Exit 0 matched / 3 changed, missing or no reference payloads / 2 operation error. [Details and measured results](BASELINE_COMPARISON.md).

For setup and the recorded live checks, see [GREASEWEAZLE_PREFLIGHT.md](GREASEWEAZLE_PREFLIGHT.md). The working shop drive is the Mitsumi on selector **B**; the original NEC has a faulty head/read path.

Windows automatically binds GW, probes, extraction and Office host-process trees to controller/operation lifetimes before execution. Closing/crashing the controller stops its supervised descendants; completed captures may resume offline while partial evidence remains intact. Normal `QUIT` still drains gracefully. New host audit records expose `controller_supervision`; old records remain compatible. No new scan flags/admin/registry changes. [Crash supervision and restart boundaries](PROCESS_SUPERVISION.md). [Next 053–075 batch commands](NEXT_SCAN.md).

## Installation and command catalog

Build with `cargo build --release`; the executable is `target\release\fluxvault.exe`. Install the current copy with `powershell -NoProfile -File .\scripts\install-cli.ps1 -AddToPath` from the repository; omit `-AddToPath` to avoid changing PATH, or add `-WhatIf` to preview. Reopen the terminal after a PATH change. **Repeat installation after a new build**: installed `fv.exe`/`fluxvault.exe` are copies. Alternatively run the full release path using PowerShell's `&` operator. [Beginner setup](TUTORIAL.md#1-install-or-refresh-the-short-command).

This is a **catalog, not a batch recipe**. Choose one relevant command from a project folder. Physical recover/scan/capture commands access the inserted floppy; do not paste the whole block:

```powershell
fluxvault status
fluxvault benchmark report
fluxvault disk list
fluxvault disk show 7 --details
fluxvault recovery plan
fluxvault recovery queue
fluxvault recovery compare 7
fluxvault recovery sector 7 --lba 0
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
fluxvault greaseweazle capture 7 --gw-drive B --source-write-protected
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
fluxvault report export --language en --json
fluxvault audit
fluxvault package build --destination C:\CustomerPackages
fluxvault package build --destination C:\CustomerPackages --keep-staging
fluxvault finalize --destination C:\CustomerPackages
fluxvault finalize --destination C:\CustomerPackages --allow-attention
fluxvault finalize status
fluxvault finalize resume
```

Use `--project C:\path\to\project` to select a project explicitly. Add `--json` to a command for machine-readable stdout (including structured errors); long-running progress goes to stderr. Exit code 0 means complete, 3 means attention/partial, and 2 means invalid input or an operation error. These codes will be refined as production automation is added.

`package build --keep-staging` additionally keeps `<package-stem>.staging` next to the verified ZIP, containing its exact members, README and manifest/hash sidecar. Each copied file is hash-checked; internal/quarantined files stay excluded. JSON adds `staging` (a path, or null without the flag). This is a separate unpacked **snapshot of the verified ZIP**, not a move, merge, live project view, or customer-completeness certificate. It requires extra storage and copy/verification time; default packaging and automatic production remain ZIP-only. Cancellation/errors retain uniquely named `.partial.staging` / `.partial.zip` evidence, never replace an earlier folder, and do not report successful packaging. If cancellation occurs after the folder is verified/published but before ZIP promotion, that verified folder can remain alongside the partial ZIP. The original project is always preserved.

`disk show N` summarizes saved attempts. Add `--details` for their hashes, bad-sector LBAs, retry counts, and evidence paths; JSON includes those fields without an extra flag. Neither view accesses the floppy drive.

`report export [--language hu|en]` refreshes audit and writes a complete versioned final bundle without tools/media or payload changes. JSON `workbook` now points to combined `FloppyFinalReport.xlsx`; `acquisition_workbook` retains the narrower Hungarian report. `final_report` includes directory/JSON/latest-pointer paths and attention counts. Scan/process also generate it and use stronger combined checks for attention. Export itself returns 0 when a warning report was successfully written. [Files and compatibility](FINAL_REPORTS.md).

## Saved-sector inspection: optional advanced diagnostic

From the project folder, `fv recovery sector 7 --lba 0` displays the first saved sector's hex/ASCII bytes, zero-based LBA, one-based CHS sector, map status, image SHA-256 and acquisition attempt. Add `--sectors 2` to inspect a small range (1–8), `--attempt 2` to choose a completed image attempt rather than the best recorded native attempt, or `--json` for structured output. This is an **image attempt**, not a raw `--capture-attempt`.

No floppy, host tool or project writer is opened. Missing/conflicting placeholders and unknown maps are explicitly labeled, never treated as recovered bytes. Reconstructed attempts trace replay-verified donor/FAT-copy origins; flux capture origins are explicitly **recorded**, hash-bound provenance rather than a fresh independent raw-flux replay. Legacy images without native acquisition metadata are not supported by this diagnostic. Exit 0 means the selected range has readable saved evidence; 3 means that range is missing/unknown/derived; 2 means invalid input or inconsistent evidence. None certifies physical label identity, the rest of the disk or customer completeness. [Bounds and examples](SECTOR_INSPECTION.md).

## USB acquisition: optional advanced path

`fv scan --usb [--drive A:] --write-blocker-verified` selects the existing USB-only scan (default A:). It now accepts the displayed number without a READ prefix; legacy READ still works. Protection checks and session-count semantics remain. This does not enable GW background processing or dual mode.

`fv scan --double --write-blocker-verified [--last-disk N]` starts the **opt-in two-drive pilot** in one console: `u1` reads USB 001, `g2` reads GW 002, and `g1` selects an available earlier USB partial for GW. Both take fresh disks; USB is fast-pass-only and GW auto-recovers. Background processing/packing continue. Commands assert exact labels/open tabs; dual rejects `--no-verify`. `u out` / `g out` confirms the last saved disk's removal; `QUIT` drains active reads. Selectors/endpoint persist. `scan --double --plan` is offline-only; `production status` inspects custody. Ordinary scan remains GW-only. [First test and limits](DUAL_SCAN.md).

Inside dual scan, `PAUSE` (`p`) persistently blocks new reads while active reads and saved-file work finish; removal confirmations remain allowed. `RESUME` (`r`) re-enables numbered confirmations without starting a read. Restart preserves a pause and requires an explicit RESUME. This is not immediate capture cancellation. STATUS includes station actions/elapsed time; offline `production status --json` distinguishes held saved partials (`usb_transfer_pending`) from removal-confirmed transfers (`usb_recovery_queue`).

Dual USB results now record automatic speed-first triage (clean -> USB complete; missing sectors -> GW, no extra USB retries). GW ACTION cues recommend the highest-ranked eligible transfer; any exact available `gN` still overrides that suggestion. `fv production queue [--json]` inspects the ranked queue offline, with reasons/hash bindings and no custody changes. Removal-confirmed disks rank ahead of still-held partials; severity/boot loss and capped later-ticket age inform the heuristic, not recovery probabilities. Old journals retain unknown age; active/interrupted custody and PAUSE still control authorization. [Policy and limits](DUAL_SCAN.md#automatic-usb-triage-and-gw-recovery-priority-2026-10-09).

Dual scan also shows current-invocation saved-label pace and a rough fresh-feed ETA after three fresh saves with an endpoint; paused feeding suppresses ETA. Old recovery transfers cannot supply fresh samples. Remaining transfers/file tail are separate, not a promised finish. `fv production benchmark [--project PATH] [--json]` replays saved dual timing events, checks receipt seals and exports immutable JSON/per-receipt CSV under `Reports/DualBenchmark`. It opens no drives/tools. New dual sessions automatically save/export these measurements (DualScan report schema 3); old runs show explicit timing gaps. Interrupted totals remain unknown, overlapping reader durations are not summed as wall time, and logs/exports are excluded from customer packages. Existing `benchmark report` is single-GW only. [Measurement details](DUAL_SCAN.md#pace-and-saved-timing-reports).

Windows `A:` is the USB floppy drive letter, **not** Greaseweazle selector A/B. This guarded path is optional and distinct from the default `fv scan` workflow.

`fluxvault drive list` enumerates removable drives without reading inserted media. `fluxvault drive probe --drive A:` opens only an enumerated drive read-only, reads at most the first 512 bytes, and reports geometry and the Windows write-protection result. A positive software result is **not proof that this USB adapter enforces physical write protection**. `fluxvault acquire --drive A: --disk N --retries 2 --write-blocker-verified` requires an operator hardware-protection assertion, a positive Windows protection report, and plausible floppy geometry; the imaging backend checks protection again when it opens the drive read-only. On 2026-09-26, Windows reported `protected` for customer floppy 007 and the CLI completed a read-only 1.44 MB acquisition. That attempt had one unresolved sector; all other 2,879 sectors matched the earlier clean archived image byte-for-byte. This validates the CLI read path, not the adapter's physical write-blocking behavior.

`fluxvault scan --drive A: --write-blocker-verified` runs a guided single-drive loop from the project's current disk number. After each swap, type the displayed number (or legacy `READ`) to image it, or `QUIT` to stop; blank/wrong-number input does not start a read. `--count N` caps this session, and `--retries N` sets the same bounded sector retries as `acquire`. A completed partial advances numbering and appears in the recovery queue; a failure does not advance it. Prompts/progress use stderr, leaving `--json` output machine-readable. The multi-disk loop itself has only been tested with synthetic acquisitions; single-disk `acquire` was tested live as described above.

## Tools and expert Greaseweazle operations

`fluxvault tools check` runs 7-Zip, LibreOffice, and Greaseweazle version checks and records executed commands in the project tool audit log (or the application audit log when no project is selected). `tools show`, `tools set NAME PATH`, and `tools clear NAME` manage per-user tool paths. `greaseweazle preview` prints safe raw-capture and file-to-file decode command examples without executing anything; `greaseweazle info` runs an audited, read-only device/firmware query. `recovery queue` shows unfinished cases; `recovery compare N`, `recovery backup N`, `recovery composite N`, and `recovery fat N` operate on saved evidence. `recovery import N --source DIR --dmde-log FILE` copies external DMDE results into guarded project recovery locations without overwriting an earlier import. `extract all` and `extract disk N` need 7-Zip, not LibreOffice. `files manifest` refreshes the recovered-file inventory. `conversion plan` builds delivery paths without LibreOffice; `conversion run` executes bounded, audited Office conversion. `conversion issues` reads saved exceptions. `conversion retry [SOURCE]` reloads the project-scoped conversion state after a restart, retries all saved issues or the selected source, and rejects changed source hashes or paths. `process` runs the existing-image recovery, extraction, conversion, audit, and workbook pipeline.

`tools check` confirms the **host program** is callable, not that a Greaseweazle board is attached. `greaseweazle info` now parses the device section and reports `device_status`/`ready` in JSON; an absent or unverified board returns attention exit code 3 even if `gw info` itself exits 0. Raw capture performs the same read-only info preflight before reserving an attempt or issuing `gw read`. This matters because upstream `gw info` explicitly prints `Device: Not found` and exits 0 in that case. [Greaseweazle info source](https://github.com/keirf/greaseweazle/blob/master/src/greaseweazle/tools/info.py)

Tool-health/version probes have short process deadlines (15 seconds for Greaseweazle and 7-Zip, 30 seconds for LibreOffice) and retain an audit entry on success or timeout. Greaseweazle host version comes from the **Host Tools** section of `gw info`, not an unsupported `--version` flag; normal output may arrive on stderr. The additional host-version probe is bounded to 10 seconds; `info`, `read`, and `convert` retain separate 15/300/60-second defaults. Automatic recovery further bounds each stage by remaining policy time.

The capture/decode path was **live-tested on 2026-10-05** with V4.1, host 1.23, firmware 1.6, the Mitsumi D353M3D-5056 on selector B, and protected WinWord 1. Tool health, board info, two full raw captures, offline decode, status, consensus, targeted automatic recovery, and downstream audit/report processing passed; one sector (LBA 24) remains unreadable. This does not validate all formats/media or prove hardware write blocking. `greaseweazle capture N --source-write-protected` runs only `gw read` with `--raw` and `--no-clobber`, preserving numbered SCP, SHA-256, metadata, and audit. The flag records your physical-tab confirmation. Format is inferred from saved 1.44 MB/720 KB USB evidence when possible; otherwise pass `--profile ibm.1440` or `--profile ibm.720`. Selector A remains the generic default; **use B for the current straight cable**. `--revs N` accepts 1–10, default 3. Selectors are not Windows drive letters.

### Automatic Greaseweazle-only recovery

```powershell
fluxvault greaseweazle recover 7 --gw-drive B --source-write-protected
```

No USB scan is required. Expert `recover` retains its **ibm.1440** default; `--profile auto` enables saved-flux discovery, or pin DD with `--profile ibm.720`. Confirm identity/protection first. Default stages are Fast (2 revolutions, 0 retries), Normal (3, 2), Recovery (5, 3), Detective (8, 5). They stop on a complete map, two non-improving passes, or configured pass/time limits. Each stage defaults to 600 seconds of capture time, not 600 shared by the whole disk; a stubborn disk can use roughly 40 minutes plus offline work. Later passes target problem/control cylinders using the selected format. No routine drive swapping or speculative repair.

Raw captures and decodes remain separately hashed. A durable per-disk journal in `Flux/Recovery` reuses a saved whole-disk decode when available, resumes saved raw evidence after decode failure, and verifies completed artifacts on repeat invocation instead of reading again. A per-project/disk lock blocks duplicate jobs for that disk. CLI `scan`, `recover`, `capture`, and `info` share an OS-held reservation at `%APPDATA%\FluxVault\.fluxvault-greaseweazle.lock`, across projects using that settings location. It releases when the owning process exits; the remaining lock file is harmless. Direct `gw.exe`, separate Windows users/settings locations, and library consumers are outside this CLI reservation. Matching controls help detect disk swaps but cannot prove identity.

### Guided Greaseweazle-only batch

```powershell
fluxvault disk select 1
fluxvault greaseweazle scan --gw-drive B --source-write-protected --count 10
```

Use the project's next unscanned number (or a fresh project). Each swap requires the displayed number (`004` or `4`); bare `READ`, a different number, or arbitrary input starts no read. Legacy `READ 004` also works. `QUIT`/`Q` or end-of-input stops feeding. `--count N` caps results finalized this invocation, including resumed numbering commits. Plain `scan` reuses saved selector/profile/map/policy/end target; fresh projects use B, automatic supported IBM 720 KB/1.44 MB discovery and the normal policy. `--acquisition-only` skips downstream work. Every numbered confirmation includes the prompt's physical write-protect check.

Each disk uses the existing bounded recovery service. A verified terminal result—including partial/unrecoverable-within-policy—advances the project number. Operation failure keeps it selected. Numbered custody, pending result and completion history are atomically recorded in the internal project-root `.fluxvault-gw-scan.json`; an OS-held `.fluxvault-gw-scan.lock` prevents duplicate project scans. Control files and partial metadata commits are excluded from packages. Do not manually edit the journal or change disk selection from another session.

On restart, a pending result is verified against the committed single-disk journal, raw/decoded evidence and published hashes before reconciling the cursor. A cursor at either the pending disk or its next number is accepted; it advances only once without a physical read. An interrupted job with no recorded result asks for the same numbered custody confirmation, then lets the single-disk journal resume/reuse evidence. Different pending settings, changed published bytes, invalid identities or cursor disagreement refuse further reads.

New scans enqueue each verified image for coalesced background extraction/conversion/audit/workbook work while feeding continues. Worker stages stay in logs so swap cues remain clear. After feeding, drain and reconcile the whole project. `processing status` is read-only; `processing resume` drains saved jobs offline and retries failed outputs once through existing bounded policy. Unchanged failures are not relaunched for every disk arrival. `--processing-mode tail` retains the serial tail; older journals default to tail. `--acquisition-only` skips downstream work. Failed work preserves evidence/jobs/numbering. JSON includes session/project results and background outcome; exit 3 is attention, not delivery certification.

Mock-executable tests cover numbered confirmations, two-disk acquisition, disconnected-board failure, bounded partial recovery, restart on both sides of the numbering commit, changed-image refusal and cross-project contention. A separately running scan is terminated in the lock test; a new session then acquires the released reservation without starting a host tool. Live 001–020, 053–064 and 021–032 cohorts now exercise the guided batch; full physical 136-disk acceptance remains separate.

### Measured pilot runs

For the first twenty numbered customer disks, see [PILOT_20.md](PILOT_20.md). `scan --profile-map FILE` accepts a regular workstation JSON file (maximum 64 KiB) of the form `{"schema_version":1,"profiles":[{"disk":9,"profile":"ibm.720"}]}`. Entries override the default/`--profile` for their disk only; formats must be exactly `ibm.1440` or `ibm.720`. Duplicate/invalid numbers, unknown fields and unsupported schemas/formats are refused. The resolved map is persisted with pending scan settings and recorded in telemetry, along with each read's chosen profile; changing it while a disk is pending is refused. Older journals without a map retain their existing default-profile behavior. This supports known mixed batches, not automatic physical format detection.

Normal GW scans preflight 7-Zip and LibreOffice before displaying a custody prompt or starting acquisition. Failures preserve the cursor and do not create a pending scan job. `--acquisition-only` explicitly skips these downstream requirements. Tools are rechecked when processing actually runs, so a later failure still retains all saved evidence. `policies/pilot-short.json` provides an opt-in three-pass, 180-seconds-per-stage acquisition policy for the small pilot; default recovery limits are unchanged.

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
  "time_limit_scope": "per_stage",
  "no_improvement_limit": 2
}
```

Limits: 1–8 passes, 30–1800 seconds per stage by default, 1–3 non-improving passes, 1–10 revolutions and 0–10 retries per pass. Resume requires the same disk, policy, profile, and selector. Failed/interrupted captures retain numbered partial evidence; only completed raw artifacts can be decoded without a new physical read.

`greaseweazle decode N` uses the latest completed SCP attempt by default, or `--capture-attempt N`, and verifies the source hash before running file-to-file `gw convert`. Derived images are kept separately under `Flux/Derived`, with their own hash and provenance, and are **not promoted into Images or treated as clean sectors**. The decoder records any `Found X sectors of Y` summary reported by Greaseweazle. For standard IBM 80-cylinder profiles, it records exact missing LBAs only if the entire reported grid is present and consistent; otherwise the map stays unknown. This is still Greaseweazle's report, not independent verification of each image sector. `greaseweazle status N` re-hashes saved raw and derived evidence, highlights missing/changed files, and shows reported sector counts/map availability without needing the board or host tool. Both decode and status return exit code 3 because sector-quality integration and delivery certification remain open. No physical drive is accessed for decode or status. `gw read --format` without `--raw` can regenerate flux rather than preserving what the disk emitted; FluxVault always pairs them for raw SCP captures, as documented by the [Greaseweazle image-type guide](https://github.com/keirf/greaseweazle/wiki/Supported-Image-Types).

`greaseweazle compare N` is an offline, read-only comparison of the best saved USB attempt and latest decoded flux image. It rechecks both image hashes and matching geometry, requires a complete Greaseweazle sector map, and reports sectors good in both but byte-disagreeing, USB-bad/flux-reported-good donor candidates, USB-only-good sectors, and still-unresolved sectors. Donor candidates are **not** automatically merged or certified; Greaseweazle's map is still vendor-reported evidence. A missing/inconsistent map, changed hash, unsafe path, or geometry mismatch stops comparison. No floppy drive or external tool is accessed.

`greaseweazle consensus N` compares the latest decodes from the two latest **distinct raw capture attempts**. It requires the same format, complete sector maps, intact raw and decoded hashes, and reports exact LBAs that agree byte-for-byte, conflict despite both being reported good, appear good in only one pass, or remain bad in both. Two decodes of one SCP do not count as independent captures. This is an evidence check, not a promoted composite or customer-delivery certification; it never accesses a floppy.

`greaseweazle plan N` combines the best saved USB attempt with decodes from the two latest distinct raw captures. It verifies paths, hashes, geometry, and complete reported sector maps before identifying USB-bad sectors where both flux images report a read and agree byte-for-byte. It separately lists one-flux-only sectors, unresolved sectors, and any USB/flux or flux/flux byte conflicts. Matching control sectors provide useful evidence but are not proof of physical disk identity. The command is offline and read-only, always returns attention code 3, and **does not create or promote a composite**.

Failed or interrupted offline decodes retain a numbered `.partial.json` attempt record (and any partial image). `greaseweazle status N` shows these as needing attention, and the next decode uses a new number instead of overwriting the failed attempt.

Office/PDF reuse requires both a matching saved source hash and a matching saved output hash, even after a restart. An older valid-looking output with no saved binding is preserved and reported as an issue rather than silently claimed as current conversion evidence. `conversion issues` reloads saved issues on every invocation.

Equivalent source bytes may move between extraction attempts after USB-to-GW recovery or generation selection. Reuse now also binds exact disk/relative identity, output paths and format/filter settings, not only the old extraction path. Already affected projects can restore missing bindings from successful same-project `Reports/ConversionHistory` records, checking actual output hashes; malformed/foreign/linked/oversized history is refused (4,096 entries, 8 MiB per record, 128 MiB search ceiling). Current bound-output hash mismatch remains a refusal, not an invitation to overwrite or adopt a valid-looking file. Run saved-only `fv process` to refresh the corrected report; historical session reports are preserved.

### Native file recovery from damaged saved images

```powershell
fluxvault recovery extract 7
```

This command needs no physical disk or external extraction tool. `extract disk N`, `extract all`, and `process` also try the same native fallback automatically for partial images, failed extraction/listing, or zero-file extraction. Those wider commands still require their normal external tools.

Native recovery requires a hash-matching saved image and recognized completed acquisition log/map. Readable directory/FAT-chain recovery supports unpartitioned FAT12 volumes with 512-byte sectors, at most 4 MiB. Geometry may come from readable BPB metadata or the warned, corroborated standard-layout fallback described in [DAMAGED_FILESYSTEM_RECOVERY.md](DAMAGED_FILESYSTEM_RECOVERY.md). With no usable layout, bounded signature recovery can search readable regions without claiming original filesystem reconstruction. Missing directories, FAT disagreement, loops, cross-links, size mismatches and unreadable content remain explicit exceptions rather than guessed bytes.

Current files are published separately under `Extracted/NNN/attempt_NNN_native_v5` (or `legacy_native_v5`), preserving earlier extraction/manual recovery. Provenance is in `Recovery/NNN/attempt_NNN_fat12_v5.json`, with source/per-file hashes, content/metadata LBAs, allocation links, skipped entries, directory gaps, name evidence and signature-candidate extents/validation. Old generations remain intact; running recovery/extraction can publish the improved generation without a new acquisition. Inventory/provenance is rechecked before reuse/delivery mirroring. Existing differing files/reports are not silently replaced.

Intact VFAT names require full sequence/checksum/type/UTF-16/padding and safe-path validation; their raw name units/entry offsets are recorded. Missing or unsafe names retain a safe recorded alias and fallback reason. Name/alias collisions are refused. Generation 4 additionally searches allocated orphan chains, readable runs inside partial files and raw readable regions when layout is unavailable. PNG/JPEG/BMP/GIF, ZIP, RTF and OLE candidates require bounded structural/content checks, retain exact offsets and use reconstructed names under `SignatureRecovery` / delivery `Signature-Recovered`. Known deleted/free allocation is excluded with a readable layout; unknown-layout carving explicitly cannot establish live/deleted status. It does not guess missing file/name bytes or certify original text/customer completeness. `recovery extract` returns **3**, including reuse. Normal processing automatically converts supported recovered Office candidates; see [CARVING_RECOVERY.md](CARVING_RECOVERY.md). Batch exceptions appear in `RecoveryExceptions.txt`; `BrokenForDMDE.txt` is a compatibility filename, not a mandatory manual-DMDE instruction.

Validated `System Volume Information`/`$RECYCLE.BIN` directory names remain available in forensic extraction/inventory but are excluded from new delivery mirrors and packages. An unvalidated `SYSTEM~1` alias is not guessed to be an OS folder. Shared same-acquisition selection now promotes verified coverage/name improvements and preserves competing generations. Original-mirror ownership enables equivalent obsolete copies to move into recoverable `Recovery/DeliveryQuarantine`; edited/untracked/pre-ledger copies and old Office derivatives remain preserved. Selection/cleanup reports explain changes and are included in customer archives; quarantine/private ownership state is excluded. See [recovery selection and delivery maintenance](RECOVERY_SELECTION.md).

Generation 5 additionally reconstructs bounded evidenced lost-parent directories (`DirectoryRecovery` / delivery `Directory-Recovered`) and structurally validated single-missing-FAT-link suffix alternatives (`FragmentRecovery` / `Fragment-Hypotheses`). Original parent/name/ownership or tail association remains unproven; competing candidates are retained, not silently certified. `recovery documents N [--json]` invokes the same native service and focuses output on forensic Word main-text salvage. Word-text engine 2 can selectively map surviving root Word/table regular/mini streams past unrelated missing CFB directory entries; missing required metadata/encryption/unsupported formats remain refusals. Exact CP/source extents, UTF-8 segments, generated offline readable HTML and the report live under `Recovery/NNN/attempt_NNN_word_text_v2`; old v1 evidence stays intact. Human output prints edition paths; JSON returns `readable_edition_paths`, `readable_editions`, `selectively_mapped_documents` and `engine_version`. Editions show explicit gaps and do not execute field codes/macros or fetch assets. No original replaced/repaired, extra whole-file count or routine Office conversion. Exit 3, including reuse; no physical media/external tools. [Full bounds and examples](DEEP_RECOVERY.md).

Incomplete file-candidate readable runs automatically preserve `.bin` fragments under `Recovery/NNN/attempt_NNN_fragments_v2`, preserving older v1 folders, with exact logical offsets, physical extents, EOF-clipped holes and hash-bound reports. They never enter whole-file counts or conversion. Optional `recovery extract N --include-deleted` searches surviving deleted chains and structurally validated free-contiguous hypotheses, publishing only forensic candidates under Recovery and leaving normal extraction/delivery unchanged. Both return attention code 3; no physical media or external tools are accessed. [Commands, evidence and limitations](DELETED_AND_FRAGMENT_RECOVERY.md).

Saved-image validation on 2026-10-06 recovered **22 forensic files / 1,208,710 bytes** from WinWord 1, preserved the original image SHA-256, and retained partial audit status for unreadable LBA 24. Generation 2 validated two long names and matched all 22 paths and hashes against independent 7-Zip extraction. A fresh `process` run produced 21 installer delivery originals, excluding one 76-byte Windows metadata file. Verified reuse and non-overwriting generation upgrade passed. There were no new physical reads and no Office conversion candidates on that installer disk. This is not a full corrupted-filesystem recovery benchmark.

`finalize --destination PATH` refreshes saved-image recovery/extraction/conversion/audit/final reports and builds/verifies an archive under one project owner and stoppable run. It requires at least one image and an existing workstation destination outside the project. Default attention blocks packaging; explicit `--allow-attention` archives partial results with reports, still returns 3, and never bypasses fatal evidence/package errors. `finalize status` inspects historical receipts offline; `finalize resume` uses saved options and rechecks current products without physical reads. Completed receipts require a new finalize for a fresh snapshot. ZIP integrity is not customer-delivery certification. [Beginner/advanced finishing guide](FINALIZATION.md).

The CLI has guided read-only acquisition, bounded IBM discovery/raw-only exception continuation, native intact-chain/layout recovery and coalesced single-GW background processing. Deeper damaged/deleted-file recovery, additional format decoding, full live acceptance and two-drive scheduling remain in [TODO.md](TODO.md).
