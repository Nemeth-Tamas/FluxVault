# Four closures — 2026-10-13, 462/480

Later scope adjustment on 2026-10-13: the operator removed automatic swap detection from the TODO, retaining explicit physical confirmations. Current count is **462/479 (96.5%), 17 open**; no extra feature was completed and no code changed. The 480-item checkpoint below is historical.

Baseline **458/480**, now **462/480 (96.3%), 18 open**. Four existing boxes closed, unchanged denominator. The ten-box target was not met; universal power-loss/custody, broader damaged-document/layout reconstruction and historical recovery parity are not replaced by narrower tests.

| Existing item | Completion contract |
| --- | --- |
| Saved sector/file read provenance | Exact managed-file trace verifies native/deleted chains and carved/fragment extents; image/attempt/hash bindings join original USB retry or GW/donor origins. Conversion output hashes recurse into input lineage. Word/RTF editions expose report-declared encoded-source relations. Manual/old unknown origins and semantic/character equivalence remain explicitly unknown. No tools, media or output writes. |
| Observed remaining-time estimate | Persisted worker-specific full processing durations and successful package byte/time observations join single-GW/dual reader/retry history. Conditional overlap/tail ranges need three comparable observations per required phase; incomplete/paused/ambiguous histories remain unknown. Quartiles are not hard bounds or a deadline. |
| English operator / HU-default customer presentation | Geometry/protection/imaging messages are English; native descriptions and customer-status explanations are localized. Existing HU/EN report, package and Word/RTF edition/resume bindings remain intact. Source content, stable codes, raw evidence and old generations are not translated. Both damaged-output language/package paths were verified. |
| Small live clean/damaged/DD/program-media smoke | Operator-confirmed protected Mitsumi/B reads of WinWord, 009 and Memtest 101 preserve captures/maps/telemetry/cursors. Offline finishing handles damaged native recovery, clean DD and zero-file program attention correctly. Prior 009 fixed-profile/map smoke and new identical DD image support the standard IBM scope. No arbitrary non-IBM-format qualification or USB hardware-write-blocker certificate. |

## Validation

- **750 routine tests pass**, 32 environment-dependent cases remain opt-in; formatting, locked all-target checking and strict Clippy pass. Actual PowerShell completion-engine acceptance passes separately.
- Eight managed-file regression cases cover intact/deleted/signature/manual/conversion/fragment/text/HTML paths, changed hashes/reports, invalid extents and device/parent/stream/label/owner refusal. Processing/packaging forecast tests cover missing history, worker mismatch, overlap, pause and torn-tail preservation.
- Release saved-batch checks in `Acceptance-20261013-e6fcb11c` trace real 007 DOCX conversion, 022 RTF HTML, 052 Word HTML and fresh WinWord native metadata/payload donor origins without source changes; old 136-disk history gives unknown whole completion rather than an invented missing processing/package estimate. Single-GW history is recognized separately.
- Fresh saved WinWord processing reconstructs one mirrored FAT sector in a separate DERIVED image and recovers 22 files with attention. 009 processing is clean. Memtest-86 v3.0/101 preserves a clean sector image but reports zero recovered files/attention. Original acquired hashes remain unchanged.
- Fresh HU/EN WinWord report/package generations both pass every ZIP member/manifest/hash verification. Prior historical root archive and original script ZIP retain their recorded SHA-256 values.

[Hardware projects, timing and immutable image hashes](../testing/HARDWARE_20261013.md). [Commands and lineage limits](../recovery/EVIDENCE_IMPACT.md). [Forecast assumptions](../guides/SAVED_BATCH_TOOLS.md). [Language boundary](../guides/CUSTOMER_LANGUAGE.md).

## Important open acceptance

Two post-gate lineage-only reruns fast-failed with `0xc0000409`. A bounded repeated run with `RUST_BACKTRACE=full` reproduced the failure on round four: parallel fixtures reused the same timestamp-only root, recovery correctly refused another fixture's operator files, then `Drop` panicked because the shared root had already been removed. The double panic aborted the harness. Exclusive root reservation plus a per-process atomic suffix fixes that new isolation defect; cleanup no longer replaces a first panic with a second one. The older storage occurrence has no matching original stack and is not declared solved by this diagnosis.

The optional USB presence watcher observed **stale PRESENT after confirmed removal**. It requests no data/write access and never changes custody or starts acquisition. Automatic swap detection was subsequently removed from the requirements; explicit confirmations remain the intended workflow. This is neither a write-protect enforcement test nor proof that cached driver metadata can identify physical labels.

The actual 136-disk benchmark's throughput/contention/yield acceptance, unexplained historical Windows fast-fail, every crash/power-loss publication boundary, deleted/carved historical-classification reconciliation and full same-evidence DMDE yield parity remain separate. A/B serial preload is still deferred for cabling; no simultaneous captures on one GW board or unconfirmed third drive is introduced.

Post-fix validation: 30 consecutive parallel eight-test lineage rounds and a single-threaded run pass, followed by the full 750-test routine gate, strict lint/check/format and release build. Final release SHA-256: `E16EB86485401AFF32E714EA51EBB2DA8F6162D4E61456BF1ECF389376201657`.
