# Snapshot candidates and damaged-file byte copies

FluxVault preserves **snapshot byte candidates** when a legacy image's acquisition log/sector map is missing, unrecognized or contradictory. It also preserves **offset-maintaining damaged-file copies** when a completed legacy map identifies data holes but the entire file allocation is coherently mapped. Both are separate forensic paths: neither labels unknown bytes readable, repairs missing data or certifies a complete original file.

## Ordinary use

From a project folder:

```powershell
fv extract all
```

The native fallback automatically saves eligible candidates in:

```text
Recovery/SnapshotCandidates/NNN/attempt_NNN_v1_SOURCEHASH_CONTEXTHASH/
    SnapshotCandidates.json
    Files/0001/candidate_original-name.ext
    Files/0002/candidate_another-name.ext
```

The source images/logs, `Extracted`, routine conversion selection, original numbering and earlier generations remain unchanged. Such disks stay in the recovery/attention list; candidate counts do **not** increase ordinary extracted-file counts. Existing operator/manual recovery is preserved. Completed reliable maps still use ordinary native recovery for intact files and readable fragments; legacy partial maps additionally retain separate damaged copies. Native acquisition metadata is not admitted to this legacy-only candidate command.

The report records measured image/log hashes, observed names, payload hashes, exact source byte extents, metadata-sector observations, structural/allocation checks, engine version, refusals and search limits. In v1 (unavailable/contradictory maps), `physical_readability` is `unknown`, `acquisition_mask_known` is false, and `bad_sector_count` is null—not zero. Forensic files may still contain historical fill bytes or corrupted original content. Structural validity and historical byte matches are not semantic/document-completeness certificates.

In v2 (known data holes), `acquisition_mask_known` is true, `known_bad_lbas`/`bad_sector_count` retain the completed source map, and each file records `known_unreadable_ranges` with logical offsets, EOF-clipped lengths and source LBAs. Full-file physical content remains unknown. Copies retain **exactly the bytes already in the image**, including existing filler/corruption in marked holes; FluxVault adds no zero fill or guessed replacement. This is not intact recovery or format validation. No readable fragments are concatenated across gaps, and final-cluster slack is excluded. Two local FAT interpretations must agree on the complete allocation/ownership; they are not independent physical reads. Unmapped tails, unavailable metadata and ambiguous/shared ownership refuse a full-length copy. Some otherwise useful damaged files remain excluded by these conservative checks.

V2 folders use `attempt_NNN_v2_SOURCEHASH_CONTEXTHASH`; v1 folder names/serialized reports remain compatible and independently replayable. Both modes share the same immutable, source/log-bound publication and reuse checks. Existing v1 candidates on actual disk 066 were reused unchanged.

Eligibility checks historical sector evidence against the **actual saved file extent**, not the catalogue's repeated log claim. A sector-aligned truncated image can supply bounded signature candidates from its present bytes; its missing tail is never padded. A coherent filesystem whose log overclaims its extent can supply snapshot-only candidates, not ordinary verified extraction. Historical labels/geometry remain untouched, the catalogue retains attention, and the evidence audit keeps size/hash warnings alongside extraction problems rather than replacing them or mislabelling them as USB/GW conflicts.

Names are parser observations: reconstructed directory wrappers, inferred layouts and name fallbacks can remain hypotheses, not original parent/name proof. The confidence of the entire candidate stays snapshot-only; exact payload extents do not upgrade those metadata interpretations.

## Advanced inspection

Preview one saved disk without publishing any output:

```powershell
fv recovery snapshot 66 --plan
```

Save or independently reverify/reuse its candidates:

```powershell
fv recovery snapshot 66
fv recovery snapshot 66 --json
```

Saving returns attention exit code **3**, even if candidates were produced successfully. Preview returns 0. Explicit disk identity, folder-first project discovery and `--project PATH` apply. No board, physical reader or external tool is used by this command. It is limited to eligible completed legacy snapshots; an in-progress acquisition, clean reliable-map image, changed binding, non-sector-aligned source or unsupported extent is refused.

Use `fv recovery trace N EXACT_PROJECT_RELATIVE_PATH --json` with the exact candidate path shown by the report. Trace replays the source analysis and verifies every generation file; it never invents an initial successful physical read, donor pass or corroboration. Wrong labels, changed files and stale/non-current generations refuse.

Context/source hashes select immutable generations. Updated historical context can create a separate generation; old evidence is preserved. Payload/report/extent modifications, missing/extra files and redirected paths refuse reuse. Source changes or cancellation before publication retain staged evidence rather than publish an incomplete success. Staged directories and private locks are excluded from packages.

## Comparison and delivery

```powershell
fv benchmark compare --baseline ORIGINAL_SCRIPT_ARCHIVE.zip
```

Ordinary comparison totals keep their original meaning. The additive JSON `snapshot_candidates` section separately matches verified candidate bytes against only the still-unmatched **in-scope** reference payloads, with one-to-one multiplicity. A single candidate cannot satisfy duplicate references. Known deleted, empty, temporary, OS metadata and tool-report reference scope remains unchanged. Image/log receipts and candidate generations are reverified before/after comparison. A candidate match is not promoted into ordinary recovery totals or certification.

The section distinguishes `acquisition_mask_known` and the number of in-scope `known_unreadable_candidates`. Even an exact historical match may include the same previously saved filler/corruption. Known-hole copies cannot satisfy ordinary intact-file matching or enter customer-certified/automatic-conversion totals. OS metadata is excluded from both payload and known-hole payload counts; edited hole reports refuse comparison even if payload bytes are unchanged.

Archival ZIPs can include sealed `Recovery/SnapshotCandidates` generations as forensic evidence. Hungarian-default/English-option package READMEs explain their limits. They never enter ordinary DOCX/PDF conversion automatically, replace originals or turn an attention delivery into a customer-certified one.

## Missing-map/extent checkpoint — 2026-10-14

In the isolated `Same-Evidence-20261014` project, the original ZIP supplied 136 identical images and 137 logs; no old recovered files seeded extraction. The missing-map/extent checkpoint saved ten v1 snapshot investigations, including one zero-candidate case:

| Disk | In-scope candidate payloads | Additional exact reference matches |
| --- | ---: | ---: |
| 020 | 8 | 0 |
| 047 | 0 | 0 |
| 058 | 5 | 5 |
| 059 | 2 | 2 |
| 065 | 3 | 3 |
| 066 | 46 | 46 |
| 074 | 50 | 50 |
| 075 | 6 | 6 |
| 112 | 1 | 0 |
| 133 | 2 | 0 |
| Total | 123 | 112 |

All 96 missing references on 066/074 now have exact byte/size matches. Ordinary matches remain **882/1,159 (76.1%)**; adding the separately labelled candidates gives **994/1,159 (85.8%) reference payloads with matching available bytes**, leaving 165 without such a match. This is an extraction/candidate replay, not full-pipeline parity, customer certification or a physical read-success percentage. Candidate payload counts exclude one empty candidate; eleven payload candidates are current-only.

Extent follow-up: disk **133** has 417,792 saved bytes (**816 sectors**) but a historical log covering **822 sectors / 420,864 bytes**. Checking the log against its own repeated count previously missed that contradiction. Automatic saved-only fallback now retains two structurally parsed compound containers (17,920 and 18,944 bytes), with exact present-image extents. Their minimal allocated container lengths do not match old reference payload hashes/sizes; no new historical exact match or original filename/semantic certificate follows. The ordinary 882 matches and 112 separate exact candidate matches are unchanged. Missing reference entries include uncertain signature-carved material; 165 unmatched references are not automatically 165 lost intact live files.

The comparator independently reverified all **273 archived image/log inputs**. Extent checkpoint receipt: `Reports/BaselineComparison-1792004828280401000-37172.json` in the isolated replay project; the earlier `BaselineComparison-1792002106531665000-30976.json` remains intact. Detailed logs: `Logs/SnapshotExtractionFinal.*`, `SnapshotComparison.*`, `ExtentExtraction.*`, `ExtentComparison.*`, and `ExtentQualityGate*.log`. [Original replay source hash, inputs and scope](../testing/SAME_EVIDENCE_REPLAY.md).

## Known-hole follow-up — 2026-10-14

Automatic extraction now adds **14 damaged-file copies / 1,921,880 bytes**, each an exact byte/size match to an unmatched historical reference, but each containing explicitly known unreadable ranges:

| Disk | Additional known-hole matches |
| --- | ---: |
| 009, 017, 022, 027, 040, 052, 111, 122, 123 | 1 each |
| 033 | 2 |
| 136 | 3 |
| Total | 14 |

Ordinary matches remain **882/1,159**; separate candidate matches increase from 112 to **126**. Combined available-byte coverage is **1,008/1,159 (87.0%)**, up from 85.8%; **151** references have no such match. There are 137 in-scope candidate payloads in total, with the same eleven current-only candidates. This is **not 87% verified recovery**: matching the old archive's filler/corruption does not recover the missing original bytes. No scope/deleted/OS/temporary rules were relaxed and ordinary intact-file/Office/delivery totals did not increase. Of 27 eligible known-map partial labels, eleven produce copies and sixteen produce bounded zero-copy investigations; metadata/ownership/tail refusals remain explicit, not guessed full-length files.

All 273 image/log inputs independently reverified again; immutable receipt `Reports/BaselineComparison-1792006916439099200-14364.json`. Automatic extraction still reports 121 extracted / 126 reused / 42 recovery-attention labels, zero manual/in-progress/zero-file dispositions. Logs: `DamagedCopyExtraction.*`, `DamagedCopyComparison.*`, `DamagedCopyV1Reuse.*`. Original ZIP and source images/logs are unchanged. Additional regression coverage checks known maps/no filling/EOF gaps, non-adjacent allocation order, missing metadata/unmapped tails/shared ownership, forged hole ranges with unchanged payloads, v1 generation compatibility, automatic native-plus-copy routing and separate excluded-temporary-file accounting. HU/EN package warnings explain retained filler and missing data.

Final gates pass **808 routine tests / 33 opt-ins**, formatting, locked all-target checking and strict lint (`Logs/DamagedCopyQualityGateFinal3.log`). Release preview/reuse/trace of saved 027 verifies its 166,400-byte copy with five explicitly marked 512-byte holes at logical offsets 15,360 / 33,792 / 57,344 / 75,776 / 94,208. Trace writes zero files and invokes no tools/media. Release comparison replays the same **882 + 126** matches, fourteen known-hole payloads, and all 273 original inputs: `Reports/BaselineComparison-1792007475092354700-28216.json`. Logs `DamagedCopyRelease*` retain those outputs. Release SHA-256: `4ddcd850839fe44dad45c7429c61d1f2cd2cd9668d1aa7dd938bd6f07bb1adc8`. The checklist remains 464/479; the larger verified-yield requirement is not closed by matching damaged snapshot bytes.

Validation includes preview/no-write, immutable reuse, missing-map/contradictory-log separation, explicitly unfinished/known-map refusal, changed sources/reports/payloads, forged rehashed extents, extra/missing files, interrupted staging, exact unknown-read provenance, executable flag/JSON contracts, automatic fallback, separate baseline multiplicity/tamper checks, and HU/EN package/staging behavior. A missing historical log does not prove the original acquisition completed; these are bounded stable-byte observations, not a new completion claim. Broader damaged-file reconstruction and full historical recovery parity remain open TODO requirements.

Extent regressions additionally cover overclaimed geometry with an otherwise coherent filesystem, a truncated image's present-byte carving/no-padding/reuse, catalogue attention, retained audit hash/size/extraction warnings, and automatic routing without invoking 7-Zip. A full gate found unexpected journals/timeouts in timestamp-only parallel `dual_scan_cli` fixtures; ten subsequent rounds pass after atomic suffixes/exclusive root admission. A later `production_workflow_cli` fast-fail did not recur in its unmodified backtrace-enabled retry; that fixture now has the same exclusive admission and panic-preserving cleanup guards. Timeout/refusal diagnostics include the exact dual fixture/journal. These guards prevent the shared-folder/double-panic class but do not establish the original cause of this production recurrence or the older Windows fast-fail.

Extent quality gates passed **801 routine tests / 33 opt-ins**, formatting, locked all-target checking and strict Clippy (`Logs/ExtentQualityGateFinal2.log`). Ten repeated parallel dual-CLI runs and five production runs passed after fixture isolation guards. Extent release build and real saved-133 preview/reuse/trace passed: two candidates, identical existing generation, 417,792 source bytes, unknown readability, zero trace file writes/media/tool access. Trace/save attention exit code 3 is intentional, not a failed candidate replay. Earlier saved-066 preview/trace remains documented by the previous logs. Extent release SHA-256: `651b6d1f449abc9630493e86d64eaa3b775b5955480723e65337cccb208bae5d`. The original script ZIP retains SHA-256 `5a002153f77523fa73766d333e58e11a0edb4016080d8c1e5ce39dd707de0cf1`.

The first full gate reproduced a Windows `disk_status_cli` harness fast-fail without a stack. Twenty diagnostic reruns passed; exclusive fixture admission/atomic names and panic-preserving cleanup were added as isolation guards, and twenty further rounds plus the complete gate pass. This does not diagnose the original storage crash or prove that this new recurrence had the same cause; the historical TODO stays open. A later short-image comparison regression was fixed: an absent optional candidate generation must not make ordinary legacy comparison require snapshot-compatible sector extents.
