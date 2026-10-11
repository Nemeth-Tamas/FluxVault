# Saved 136-disk review — 2026-10-11

The working baseline is `C:\Users\User\Desktop\FluxVault-Test\Customer-136-Dual-20261010`.
Normal development now uses its saved IMG/flux evidence; another physical scan is not the default test.

## What is ready

All 136 labels have images and completed archival reports. The report checks 107 disks as OK within its recorded-evidence scope and retains 29 attention results. There are 1,120 recovered source records, 807 successful conversions, 807 modern documents and 807 PDFs, with zero invalid outputs across 1,614 generated-output checks. This does not certify every historical original or document's semantics.

Production/finalization now automatically put a verified ready ZIP/checksum and `FINAL_*` reports beside `project.json`. `DELIVERY_READY.json` names and hashes the published archive edition. Earlier archives/report editions remain preserved. For an existing completed archive, `fv package publish` verifies and copies its exact embedded report generation without reprocessing or rebuilding.

The original completed archive's hash is `6006724692a2f1d6792032a8245ce417f1c13491f61fb5946297af095a9fad95`; its ready copy was independently checked. A later finalization produces a new timestamp/hash, not a replacement for that original.

Fresh offline finalization completed with attention on 2026-10-11. Beside `project.json`, open `Customer-136-Dual-20261010_20261011_134908_381.zip` and `FINAL_FloppyFinalReport-e674184adb051b8c.xlsx`. The ZIP is 1,336,138,591 bytes, SHA-256 `e674184adb051b8cdc45303039356691833190d6d9983a64671e4f6b12437858`. An independent check verifies the root archive and every copied report/checksum against `DELIVERY_READY.json`. Existing archives, captures and historical report editions remain preserved.

## Measured full historical comparison

Reference: `TextilMuzeum_Floppy_Archive_20260920_115210.zip`, SHA-256 `5a002153f77523fa73766d333e58e11a0edb4016080d8c1e5ce39dd707de0cf1`.
The verified comparison covers all 136 acquired labels and writes immutable `Reports/BaselineComparison-*.json` / `.csv` snapshots. The corrected-provenance run is `BaselineComparison-1791718494668116700-36828`.

| Default comparison scope | Count |
| --- | ---: |
| In-scope historical payload candidates | 1,159 |
| Byte-for-byte matches, including renamed identical bytes | 985 |
| Same normalized path, changed bytes | 12 |
| Historical candidates absent from current comparison | 162 |
| Current-only payloads | 80 |
| Confirmed deleted reference entries excluded by default | 167 |
| Reference recovery-tool reports excluded | 70 |

The 162 absent candidates split into 103 DMDE-filesystem-origin entries, 54 signature-carved entries and five ordinary filesystem-origin entries. Some reconstructed/raw paths and carved candidates may be fragments or uncertain historical versions, not complete live files. Different source-image hashes also prevent a same-evidence yield claim. Do not describe these numbers as 162 proven lost live files, or conversion success as recovery parity.

Root-level `$Raw/...` folders were previously mislabelled as ordinary filesystem origin. The comparator now recognizes exact raw-wrapper directory components at the root or nested, case-insensitively, without classifying filename lookalikes as carving. This changes provenance attribution, not matching totals or deleted scope.

Largest measured absent groups: 133 (27), 027/040 (21 each), 030/121/122 (11 each), 022 (10), 033 (9) and 023 (8). Use the per-file rows to distinguish actually incomplete live chains from raw/reconstructed historical candidates before changing recovery strategies. Full comparison is now measured; matching/exceeding the historical recovery yield remains open.

## Five zero-live-file disks

| Disk | Observed saved evidence | Interpretation |
| --- | --- | --- |
| 101 | Boot/program code; literal `Memtest-86 v3.0` marker; no valid ordinary FAT12 BPB; native carving emitted no candidate; image hash equals the historical image | Not an ordinary FAT file disk. Refusing an invented FAT layout is correct. Boot-program completeness/other-format decoding is not certified. |
| 041 | Readable FAT12, zero reachable live files, no recorded directory gaps/skips, 223 deleted-entry markers | Deleted entries exist; not a failed physical read or proof of 223 recoverable deleted files. |
| 047 | Same bounded live-file result, 176 deleted-entry markers | Keep deleted recovery opt-in. |
| 103 | Same bounded live-file result, 224 deleted-entry markers | Keep deleted recovery opt-in. |
| 116 | Same bounded live-file result, four deleted-entry markers | Keep deleted recovery opt-in. |

None contributes expected default-scope payloads in the historical comparison. Do not infer why files were deleted or claim no useful deleted/orphan data exists. Updated reports expose inventory-bound native filesystem status and marker counts. They distinguish unsupported/no-reachable-live cases, filesystem completeness and derived recovery from physical read damage; attention remains rather than silently calling disks healthy.

## Five outputs outside the latest selected report binding

These are preserved earlier managed-generation products, not unexplained byte mutations. The current report still flags them as untracked **in its current selected generation**; this investigation does not silently adopt them into current conversion/source counts.

- 085: `Signature-Recovered/carved_0000438e_05f901649f78.jpg`, 3,397 bytes, SHA-256 `05f901649f785e75f70f5b5d9cb035c5622d48549ee72d039202ac511c40f227`. It matches the old `attempt_001_native_v5` inventory, source-image/native-report hash bindings and a program-owned mirror in `Reports/DeliveryMirrorHistory/efd20cdcffb1c229d5ee1e947bfcb5dada19c024be16232daca63ac7850ecc53.json` (the ledger's actual hash matches its name).
- 131: the old source `carved_00033200_83d81e879a8a.doc` has SHA-256 `83d81e879a8acde6bd46bddcacc4d908a9a696b9e0c846747ea187a328a6a70f`. Its DOCX/PDF hashes match recorded history: `17cb120673140f896d99d31b33fe762f22e706dfbb0cf2e2298584e97904d104` / `b5e8558918570f03af6aef570fa060e7cb80d96d7a411120aba028ea956ddc45`.
- 131: the old source `carved_00050c00_9184586e7ee5.doc` has SHA-256 `9184586e7ee55ec5a6fd16f8bd6e9746b0a80cc7a48963b573fb61ab38bca906`. Its DOCX/PDF hashes match recorded history: `20662276fa15d6470b1d0e3fd90732e7ce5ed2f2a2b948362bc6a94d5a38d57d` / `63c828b59b03f55961c4349df1e62f143dcdb892b96de4dc399f30e4c830120e`.

Both older 085/131 source images and embedded native-report hashes verify. All four 131 generated hashes are recorded in `Reports/ConversionHistory/PreviousConversionState-1791652809341925100-11260.json`. Later preferred acquisition/extraction selection explains why the older paths no longer belong to the current source/conversion inventory. Historical ownership/hash identity does not establish live/deleted status or document semantics. No file was retired; broader automatic historical-output attribution stays within the existing provenance/reporting backlog.

## Damage work that still earns its time

The saved native impact inspection maps 13 known live-file dependencies and 16 incomplete/ambiguous chains, including disks 017/022/027/033/040/052/066/123/128/133. Some files have metadata dependencies rather than wholly unavailable payloads. Existing forensic fragments and Word text/HTML editions remain separate from intact-original counts.

Disk 133 still has two USB/GW disagreements at LBAs 17/773 and a dependent `V_xB5MOSANG.DOC` payload. The operator confirmed the same physical label. Preserve both byte editions and inspect exact metadata/file offsets; no unsupported winner, guessed bytes or clean-customer certificate follows.

### Focused saved-image recovery — 2026-10-11

The subsequent content/font slice increases selected saved 022 `nka261056994.rtf` from 330 to **5,318 readable UTF-16 positions**, with **18 uninterpreted symbol glyphs / 19 separate text segments**. It stops at byte 143,867 on a control token cut by the 143,872-byte readable prefix boundary. Attempt 1 separately yields 3,583 positions within its 73,728-byte prefix. No state is guessed across a missing sector. Saved 128 `vasalók.rtf` has validated binary Word content: Word-text v4 yields **683/939 main-text positions**, two segments and an explicit 256-position gap despite the extension. Actual release publication/reuse preserves acquisition images, native proof/inventories and earlier editions; outputs are forensic, not intact originals. Whole-file and completed-archive/baseline counts above remain unchanged. The old completed ZIP is still its historical snapshot; new editions are in separate Recovery generations.

Disk 052's incomplete `fejléc_logoval.doc` now yields 1,792/1,792 declared main-text positions with Word-text engine 3. The old v2 refusal and saved image remain unchanged. Two independent blockers were addressed: unused storage-object fields are not stream allocations, and a fully unavailable extension-count field need not prevent source-bound common-prefix text salvage. The effective Word version/complete FIB stay explicitly unknown; all required common text-mapping fields and CLX/allocation evidence must remain readable and consistent. Text/HTML carry forensic warnings, not editable-document repair or extra intact-file counts. The completed archive and baseline statistics above are the earlier immutable snapshot; publishing v3 evidence does not retroactively change them.

## Timing acceptance

The first recorded dual session began at `2026-10-10T14:01:06.179Z`; the original verified archive finished writing at `2026-10-10T18:41:59.3497428Z`: approximately **4 hours 41 minutes**, including gaps/restarts/hotfixes. This meets the six-hour elapsed target for this observed 136-label attention archive, not a claim of an uninterrupted single-build or zero-intervention run. Recorded feeding invocations total about 3 hours 20 minutes; the final finishing invocation reported 1,120.4 seconds. These intervals have different boundaries and must not be substituted for each other.

Telemetry records 169 read starts, 158 saved receipts, 11 failed-read events, three finished invocations, one identity release and one same-disk confirmation. Receipt/read counts include transfers/retries, not 169 unique labels. Explicit removal events do not count every physical swap, so a minimum-touch benchmark remains open.
