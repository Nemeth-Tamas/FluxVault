# FluxVault progress — 2026-10-06 offline preparation

The routine operator path is `fv init` -> `fv scan` -> swap/confirm until finished. New scans identify supported IBM formats, losslessly pack verified raw captures, and continuously extract/recover/convert/audit saved images in the background. Unrecognized formats become explicit raw-only attention records rather than stopping numbered feeding. Existing projects retain their saved settings. This checkpoint uses saved captures and mock hardware only; the next hardware session should smoke-test overlapping work before scaling up.

## How close?

| Measure | Current checkpoint | Meaning |
| --- | --- | --- |
| Routine replacement of the seven scripts in `G.zip` | approximately 90% | Acquisition, extraction eligibility/manual preservation, manifests, bounded conversion, audit/workbook and verified archives are implemented, with continuous saved-file processing. This is engineering coverage, not proven equality of all historical reports/files. |
| Single-Greaseweazle 136-disk benchmark readiness | approximately 95% | Real saved-evidence processing and deterministic 136-job/scan soaks pass; numbered/Enter swaps, limits, resume, telemetry, raw-only exceptions and packed storage exist. Live overlap and full-cohort acceptance remain the final readiness gate. |
| Whole TODO list | 298 / 441 = 67.6% checked | Literal checkbox count, including nested checkpoints, umbrella tasks and historical groundwork; not a weighted product-completeness score. |
| Fully autonomous recovery/product ambition | approximately 65% | Routine acquisition and durable downstream scheduling work. Severe filesystem damage/carving, additional format decoders and two-station custody/resource control remain substantial work. |

These estimates describe implementation coverage, **not** customer-file recovery rates or solvable-disk percentages. No matching-yield percentage is supportable until the 136-image/1,667-file script+DMDE baseline is compared. Script compatibility is distinct from automating the manual recovery the scripts deliberately delegated to DMDE.

## Continuous processing and raw-only continuation checkpoint

- New scans use durable image-hash-bound jobs and one coalesced background pipeline. Swapping remains unobscured; status and elapsed stage events are saved. `processing status` is read-only, and `processing resume` drains/reconciles saved work without a floppy.
- Whole-project writer ownership excludes competing extraction/conversion/report mutations, including after a worker exits while the scan is still feeding. Short publication snapshots release during long Office work. Requested conversion workers are capped to leave two logical CPUs available; this is conservative static budgeting, not adaptive memory/I/O scheduling.
- Completed jobs retain individual disk verification state. Unchanged failed conversions are not relaunched on every disk arrival; explicit offline resume/process/retry performs bounded retries. Queue and capture-packer drain races are fixed and regression-tested.
- Ambiguous automatic IBM trials retain one verified whole-disk capture plus a bound format decision, then advance custody as a raw-only exception. No geometry, complete image or zero-missing-sector claim is invented. Packed restart needs no hardware call; decoder execution failures still stop rather than being misclassified as unsupported formats. Status, queue, audit, process/finalize and benchmark records preserve the exception as attention.
- Real validation on `C:\Users\User\Desktop\FluxVault-Test\Offline-Background-Validation-20261006`: all 20 saved images processed in two coalesced runs; all 172 fresh conversion jobs succeeded; final audit 17 verified / 3 attention; every original image hash unchanged. Approximately 8 minutes 42 seconds including copying and final checks, **not physical acquisition throughput**. A later CLI resume reconciled all records to 17 processed / 3 attention, zero pending jobs.
- An explicitly run executable test exercised the complete default background scan, drain and packing path using mock Greaseweazle and real saved-file tools. The deterministic coalescing soak covers 136 arrivals, duplicate enqueue, interruption, failed tasks, changed evidence and competing writers.
- Verification: 232 regular tests pass with no failures; 11 environment-dependent tests are excluded from the routine suite. The real background 20-disk test and the mock-hardware/real-tools CLI test were additionally run explicitly and passed. Formatting and all-target checking are clean; the release executable is rebuilt.

Operator details: [BACKGROUND_PROCESSING.md](BACKGROUND_PROCESSING.md), [CHEATSHEET.md](CHEATSHEET.md) and [PILOT_136.md](PILOT_136.md).

## Damaged-filesystem starter checkpoint

- Native generation 3 can consider a standard 720 KB/1.44 MB layout when boot metadata is unavailable, but requires matching readable FATs/root metadata and a size-consistent live root file. Contradictory surviving BPB fields are refused. Reports explicitly label the layout as an uncertified hypothesis, not reconstructed boot bytes or exhaustive proof against custom layouts.
- Skipped files retain logical byte-offset/source-LBA holes, clipped final-sector lengths and unmapped tails. No partial payload, guessed source LBA or complete-file hash is invented.
- Preserve prior native generations/operator files; warnings remain available on verified reuse in human CLI output and JSON. Existing partial/failed-extraction routing uses the new engine without additional scan policies.
- Saved 005/007/009/012 preserve 34 identical file hashes/paths/extents after boot loss simulated **in memory only**. 017 is conservatively refused in that simulation because its populated root has no end marker; its readable original BPB continues recovering 15 complete files.
- A copied historical 009 is now analyzed rather than failing at its missing BPB: 954 bad sectors, nine live-JPEG holes totaling 4,608 bytes, 30 deleted slots. No complete payload is claimed from that image. This is a diagnostic capability improvement, not an increase in proven recovered-file yield.
- 219 regular automated tests passed, plus the explicitly run saved-pilot regression; eight other environment-dependent checks were not run. Formatting/all-target checking are clean and the release executable was rebuilt.

Details and validation paths: [DAMAGED_FILESYSTEM_RECOVERY.md](DAMAGED_FILESYSTEM_RECOVERY.md). Those results remain the foundation for the continuous processing checkpoint above.

## Previous offline preparation batch

- Conservative IBM 1.44 MB/720 KB format discovery from immutable whole-disk flux, with saved candidate hashes/sector maps/decision reports. Explicit fixed formats/maps remain available. At that earlier checkpoint ambiguity stopped custody; the current raw-only exception path safely continues feeding.
- Persist selected format and automatic/fixed mode through recovery restart. Correctly distinguish per-profile decode attempt numbers, use the selected profile for comparisons/default re-decodes, and record actual format in telemetry.
- Convert-only offline backend: saved-capture identify/decode cannot query the board, even for host version discovery. The installed distribution's bounded local `VERSION` file is used when available; missing version stays unknown.
- One background lossless packer, durable task files, offline `storage resume`, capture ownership locks, atomic no-overwrite publication, verified decompression, explicit/managed retention, capacity checks, isolated materialization and archival export of packed evidence.
- Expanded mock/unit/executable coverage for automatic DD, restart/absent-board reuse, default packing, logical hash preservation, damaged archives, ownership, publication restart and export.
- Updated CLI help, cheat sheet, policy explanation and fresh 136-disk runbook. Original 20-disk runbook is clearly historical; do not rerun its `init` block.

## Real saved-evidence checks

Prior preparation batch verification: 209 automated tests passed (8 environment-dependent tests were ignored), `cargo fmt`/all-target checking were clean, and the release executable was rebuilt. Final suites were rerun after the transient harness failure noted below. Current verification counts are in the continuous processing checkpoint above.

| Check | Result |
| --- | --- |
| Saved 007 / 009 discovery | 1.44 MB / 720 KB correctly selected; 009's alternate HD decode reported zero readable sectors |
| All 26 complete captures on a separate validation copy | 1,032,982,606 original bytes -> 243,529,973 packed bytes; 76.42% saving, approximately 27.07 seconds |
| Integrity after copied raw retirement | 20/20 disk evidence inspections hash-healthy; no uncompressed SCPs needed in the validation copy |
| Packed 007 host re-decode | Same image SHA-256: `ca3831409d0ef217dc732078be99fe52ee211ddd9fc35f98cd0a22040b72ab77` |
| Real packed-evidence archival export | 135 source members, including all 26 containers/bindings and derived evidence, individually hash-verified; final ZIP SHA-256 `040dacce4385f8bf679eb3eaa1ba5b7ba685e1d88c55b2a6b3935267b8b83439` (evidence-only validation copy, not a complete customer package) |
| Source pilot evidence | All 26 raw SCPs remain in the original pilot; validation retirement was only on copied captures and is reversible through the verified ZIP members |

Validation workspace: `C:\Users\User\Desktop\FluxVault-Test\Offline-Packed-Capture-Validation-20261006`. The original pilot and script archive remain the comparison baseline. Compression conserves physical flux evidence rather than substituting reconstructed/perfect flux or sector images. No new acquisition-throughput figure is inferred from these offline results.

## Next acceptance priorities

1. Short live 007/009 and damaged-disk checks with processing/packing overlap, then a fresh numbered 136-disk benchmark folder. Measure contention, memory, yield and total operator time rather than extrapolating offline tests.
2. Compare saved pilot payloads against reachable versus deleted/orphaned/carved baseline content; implement evidence-bounded recovery for the actual yield gaps and add decoders for confirmed nonstandard formats.
3. Forced interruption/disk-full/cross-process storage soak tests, immediate coordinated tool cancellation and cleanup of abandoned private temporaries. One concurrent Windows test-harness fast-fail was not reproduced on subsequent complete reruns; it remains an investigation item, not a claimed fix.
4. Adaptive shared resource limits and eventual concurrent USB/GW custody scheduling. The single-GW background worker is implemented, not a claim of a finished dual-station production scheduler.
