# Saved-batch visibility and optional expert tools

The everyday workflow does not change: `fv production start --last-disk 136`, physical swaps/confirmations, then automatic finishing. None of the commands below is a required extra step for ordinary IBM 720 KB/1.44 MB production.

## Look at a batch

### Independent saved-flux and historical candidate checks

```powershell
fv recovery crc-check all --json
fv benchmark review 9 --baseline 'C:\archives\Original.zip' --json
```

Both inspect saved evidence only, invoke no hardware/host tool, and change no preferred image or delivery. CRC checks qualify exact observed GW bytes; USB-only labels are not applicable and missing/unconfirmed/conflicting sectors stay separate. [CRC scope / optional strict composite](../recovery/FLUX_DIAGNOSTICS.md). Historical review classifies live-chain matches, tool reports, tiny stubs, uncertain signatures and allocated compound-stream differences; a raw span containing unreadable placeholders is not an intact file. [Measured four-case review](../testing/LEGACY_CASE_REVIEW.md). Exit 3 is expected investigative attention.

Direct material expert commands now refresh audit/workbook automatically after changes, even partial failures. Read-only/help/plan/no-op commands do not. If interrupted, `status` / `processing status` exposes `audit_refresh_pending`; run `fv process` to reconcile saved state before relying on current reports. Existing finished archives/root handoffs remain their original historical generation; report refresh does not silently rebuild delivery.

Estimates separate fresh, USB-to-GW recovery and retry observations and known conversion/retry jobs. Legacy events lacking identity remain unknown rather than mixed into fresh timing. A conditional whole-batch scenario is available only after comparable processing and packaging history exists.

From your acquisition project's folder:

```powershell
fv production benchmark
fv production estimate
fv recovery ranges 59
fv recovery ranges 133 --attempt 1 --json
```

`production benchmark` adds per-label operator-action accounting to its immutable saved timing report. Read/retry/removal confirmations and identity exceptions remain separate. Every recorded project label gets a row, even when older observations are unavailable. The physical insert/remove target is two actions for ordinary fresh media, four with a USB-to-GW transfer. Command counts are not measured hand movements; implicit swaps and unlogged actions remain unknown. Neither timings nor reports authorize moving a disk still marked READING.

`production estimate` is read-only. Single-GW and dual saved histories supply observed read/recovery timing; single-GW samples are whole per-label recovery durations, not separately measured internal retries. At least three comparable observations are required per outstanding phase. Reused conversions do not supply fresh-job samples.

New background processing records complete per-batch extraction/recovery/conversion/audit durations in `Logs/PerformanceHistory.jsonl`, grouped by conversion worker count. Successful package creation records verified input bytes and elapsed time. Packaging volume is projected from the last observed bytes per saved label. These local timings contain no document content and are not integrity certificates.

Where all required history exists, the whole-batch scenario accounts for reader/processing overlap and a packaging tail. Its empirical quartiles are **not hard bounds or a deadline**. Different damage, file yield, hardware contention, swaps, capture packing, handoff copying and storage speed can invalidate it. Sparse history, unknown range endpoints, changed worker counts, paused jobs or ambiguous single/dual histories keep the whole estimate unknown. Existing projects need new completed work to warm up missing timing history. Do not add the displayed phase ranges together.

`recovery ranges` hash-checks a selected image and its map/lineage, then prints exact contiguous LBA and image-byte ranges for missing versus disputed observations. Its JSON also includes known file dependencies, incomplete chains, holes, unmapped tails and unknown ownership. Missing placeholder bytes are not recovered zeros. Disputed bytes are not silently given a winner. Unknown maps/layouts remain unknown; unavailable in this selected image does not mean globally unrecoverable from all evidence.

Normal `process`/background processing now automatically saves immutable attention-disk range snapshots in `Reports/RecoveryRanges`. Repeats verify/reuse the same content-hashed snapshot. Edited/redirected published files refuse instead of being overwritten. These are technical evidence reports, not translations or customer completeness certificates. No extra range command is necessary for automatic processing.

## Compression measurements

```powershell
fv storage benchmark 7
fv storage benchmark 5 --capture-attempt 2 --json
```

The default uses the largest verified capture; the optional attempt selects an exact complete capture, including a targeted pass. ZIP/Deflate levels 1 and 6 are measured against the identical source bytes, with full decompressed SHA-256 verification. JSON includes elapsed time, host-process CPU and 20 ms sampled resident-memory observations for compression and verification. Measurements include any other threads in that process, not child tools or a hard memory peak. No source is compressed in place or retired. This is a sample benchmark, not a guaranteed whole-project saving. Raw flux preserves evidence the old sector-only scripts never captured.

## Reproduce the original archive's counts

```powershell
fv benchmark legacy --baseline 'C:\Archives\Original-Archive.zip'
fv benchmark legacy --baseline 'C:\Archives\Original-Archive.zip' --json
```

The JSON now separates historical CSV classifications from independently parsed logs and replays the supplied final-audit script's generated-file integrity gate. For the original customer archive, these are 94/42 imaging and 83/53 final; the old 86/50 summary matches a counterfactual with that integrity gate omitted. The original final workbook is absent, so its exact generation history is not inferred. Per-label status sources, actual image hashes, disagreements and invalid generated members remain visible. This does not recertify old files or establish fresh DMDE yield parity. [Exact reconciliation](../testing/LEGACY_AGGREGATES.md).

No project or extraction is required. The ZIP is streamed in place: every member's size, CRC and SHA-256 are checked, unsafe/duplicate members refuse, and a final archive hash detects changes. Original manifest rows are bound to archived source-member paths/sizes; successful conversion claims must have archived outputs. It reports source, extraction, conversion and index counts with discrepancies preserved.

For the supplied archive this reproduces **1,667 source rows and 1,667 extracted members**, plus **972 conversion rows: 953 OK, 12 PARTIAL, seven FAILED**. These include the original tool/metadata/deleted/carved scope. They are not a claim that today's automatic recovery recreated all those files from different physical captures. The existing `benchmark compare` remains the byte/yield comparison command.

## Advanced only: other GW definitions and selected tracks

Normal scanning still auto-selects supported IBM formats. Expert operations use a separate `Flux/Expert` namespace, never advance the ordinary scan cursor, never replace normal captures/images, and never become automatic donor sectors merely because their byte count looks plausible.

Preview a physical command without starting a tool or touching a drive:

```powershell
fv greaseweazle expert capture 1 --format ibm.360 --cylinders 0-39 --heads both --step 2 --plan
```

An actual capture additionally requires `--source-write-protected`; selector B is the expert default. Qualify the drive/cable/media format first. Double stepping is an expert physical interpretation, not a universal recovery setting. Physical operations remain raw, read-only, bounded and board-reserved; no write/erase/clean command is exposed.

Decode a saved SCP in an **isolated test project**:

```powershell
fv greaseweazle expert decode 9 --format ibm.720 --input 'C:\Evidence\009.scp' --expected-bytes 737280
fv greaseweazle expert decode 9 --format fv.customer.dd --diskdefs 'C:\Definitions\expert-dd.cfg' --input 'C:\Evidence\009.scp' --expected-bytes 737280
```

The definition name is passed to the installed GW host rather than hardcoding every format in FluxVault. Optional self-contained UTF-8 `--diskdefs` files and the inspected offline SCP input are copied/hash-bound into the immutable operation directory. The host reads those snapshots, not a mutable external source; both snapshots and the original SCP are rechecked before committing output. External definition includes/imports refuse because their dependencies are not bound. The shipped `scripts/fixtures/expert-dd.cfg` demonstrates a separately named DD definition. Upstream GW decides whether the requested definition can produce an IMG; additional automatic format discovery/filesystem decoders remain separate.

`--cylinders` accepts distinct numbers/ranges within 0–81; `--heads` is `0`, `1` or `both`; `--step` is `1` or `2` and cannot select physical cylinders past 81. `--revs 1..10` and `--gw-drive A|B` are capture-only. Decode requires a saved SCP and explicit `--expected-bytes 1..16777216`. Input/raw output is bounded to 128 MiB, definitions to 256 KiB, and host runs to ten minutes. Decoder failures, wrong output sizes or changed source/definition preserve partial evidence. A successful expert result returns **attention exit 3**, not clean acquisition/certification; source/output hashes and command/version are recorded. Offline decode never probes a board, even for a version.

The command syntax is grounded in the installed host and [GW read](https://github.com/keirf/greaseweazle/blob/master/src/greaseweazle/tools/read.py), [convert](https://github.com/keirf/greaseweazle/blob/master/src/greaseweazle/tools/convert.py) and [track-selection implementation](https://github.com/keirf/greaseweazle/blob/master/src/greaseweazle/tools/util.py). Passing a requested size is an operator assertion, not independent CRC, original geometry or file completeness proof.

## Developer-only performance/acceptance recipes

```powershell
.\scripts\benchmark-conversion.ps1 -Project 'C:\Archives\Saved-Pilot' -FileCount 24
.\scripts\validate-saved-closure.ps1
```

The first copies hash-bound saved files into new isolated projects and runs fresh 4/8/12-worker conversions, checking output hashes/containers and sampled owned-process-tree CPU/RSS. It never reconverts the customer's existing outputs. One trial per worker count is not a universal best preset; four remains the conservative default.

The second is explicitly tied to the known saved 136/20-pilot/archive acceptance dataset, with path overrides available. It verifies archive inventories, per-label actions, range diagnostics, three compression classes and real offline custom/DD/360 interpretations, then rechecks protected acquisition/handoff bytes. It does not read physical media. [Measured results and the exact fifteen closures](../development/CHECKLIST_450.md).
