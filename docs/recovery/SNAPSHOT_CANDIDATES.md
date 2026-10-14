# Files from saved snapshots with missing acquisition metadata

FluxVault can now recover **snapshot byte candidates** when a legacy image exists but its acquisition log/sector map is missing, unrecognized or contradictory. This is a separate forensic path: it does not label unknown sectors readable or certify a complete original file.

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

The source images/logs, `Extracted`, routine conversion selection, original numbering and earlier generations remain unchanged. Such disks stay in the recovery/attention list; candidate counts do **not** increase ordinary extracted-file counts. Existing operator/manual recovery is preserved. Completed reliable acquisition maps continue through ordinary native recovery instead.

The report records measured image/log hashes, observed names, payload hashes, exact source byte extents, metadata-sector observations, structural/allocation checks, engine version, refusals and search limits. `physical_readability` is `unknown`, `acquisition_mask_known` is false, and `bad_sector_count` is null—not zero. Forensic files may still contain historical fill bytes or corrupted original content. Structural validity and historical byte matches are not semantic/document-completeness certificates.

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

Saving returns attention exit code **3**, even if candidates were produced successfully. Preview returns 0. Explicit disk identity, folder-first project discovery and `--project PATH` apply. No board, physical reader or external tool is used by this command. It is limited to eligible completed legacy snapshots; an in-progress acquisition, reliable-map image, changed binding, non-sector-aligned source or unsupported extent is refused.

Use `fv recovery trace N EXACT_PROJECT_RELATIVE_PATH --json` with the exact candidate path shown by the report. Trace replays the source analysis and verifies every generation file; it never invents an initial successful physical read, donor pass or corroboration. Wrong labels, changed files and stale/non-current generations refuse.

Context/source hashes select immutable generations. Updated historical context can create a separate generation; old evidence is preserved. Payload/report/extent modifications, missing/extra files and redirected paths refuse reuse. Source changes or cancellation before publication retain staged evidence rather than publish an incomplete success. Staged directories and private locks are excluded from packages.

## Comparison and delivery

```powershell
fv benchmark compare --baseline ORIGINAL_SCRIPT_ARCHIVE.zip
```

Ordinary comparison totals keep their original meaning. The additive JSON `snapshot_candidates` section separately matches verified candidate bytes against only the still-unmatched **in-scope** reference payloads, with one-to-one multiplicity. A single candidate cannot satisfy duplicate references. Known deleted, empty, temporary, OS metadata and tool-report reference scope remains unchanged. Image/log receipts and candidate generations are reverified before/after comparison. A candidate match is not promoted into ordinary recovery totals or certification.

Archival ZIPs can include sealed `Recovery/SnapshotCandidates` generations as forensic evidence. Hungarian-default/English-option package READMEs explain their limits. They never enter ordinary DOCX/PDF conversion automatically, replace originals or turn an attention delivery into a customer-certified one.

## Measured historical replay — 2026-10-14

In the isolated `Same-Evidence-20261014` project, the original ZIP supplied 136 identical images and 137 logs; no old recovered files seeded extraction. Automatic extraction now saves nine snapshot investigations, including one zero-candidate case:

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
| Total | 121 | 112 |

All 96 missing references on 066/074 now have exact byte/size matches. Ordinary matches remain **882/1,159 (76.1%)**; adding the separately labelled candidates gives **994/1,159 (85.8%) reference payloads with matching available bytes**, leaving 165 without such a match. This is an extraction/candidate replay, not full-pipeline parity, customer certification or a physical read-success percentage. Candidate payload counts exclude one empty candidate; nine payload candidates are current-only.

The comparator independently reverified all **273 archived image/log inputs**. Its immutable receipt is `Reports/BaselineComparison-1792002106531665000-30976.json` in the isolated replay project. Detailed logs: `Logs/SnapshotExtractionFinal.*`, `SnapshotComparison.*`, and `SnapshotQualityGate*.log`. [Original replay source hash, inputs and scope](../testing/SAME_EVIDENCE_REPLAY.md).

Validation includes preview/no-write, immutable reuse, missing-map/contradictory-log separation, explicitly unfinished/known-map refusal, changed sources/reports/payloads, forged rehashed extents, extra/missing files, interrupted staging, exact unknown-read provenance, executable flag/JSON contracts, automatic fallback, separate baseline multiplicity/tamper checks, and HU/EN package/staging behavior. A missing historical log does not prove the original acquisition completed; these are bounded stable-byte observations, not a new completion claim. Broader damaged-file reconstruction and full historical recovery parity remain open TODO requirements.

Final quality gates pass **796 routine tests / 33 opt-ins**, formatting, locked all-target checking and strict Clippy. Release build and real saved-066 release preview/trace pass; trace reports zero media/tool access and zero file writes. Release SHA-256: `42f284818af37ef6af83e8ae4b7c7c2db30298bb9434d5d20e7b51a5eb64c6ee`. The original script ZIP retains SHA-256 `5a002153f77523fa73766d333e58e11a0edb4016080d8c1e5ce39dd707de0cf1`.

The first full gate reproduced a Windows `disk_status_cli` harness fast-fail without a stack. Twenty diagnostic reruns passed; exclusive fixture admission/atomic names and panic-preserving cleanup were added as isolation guards, and twenty further rounds plus the complete gate pass. This does not diagnose the original storage crash or prove that this new recurrence had the same cause; the historical TODO stays open. A later short-image comparison regression was fixed: an absent optional candidate generation must not make ordinary legacy comparison require snapshot-compatible sector extents.
