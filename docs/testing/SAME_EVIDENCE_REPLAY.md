# Same-evidence extraction replay — 2026-10-14

This run removes the old/new-capture confound from the script/DMDE comparison. It does not import DMDE's recovered files and call them automatic recovery. It also does not certify full parity: the observed extraction-only result remains below the reference.

## Reproducible workflow

Use the original saved script ZIP and an absent destination outside existing projects. Do not initialize the destination first:

```powershell
fv project import --source ORIGINAL.zip --destination NEW_FOLDER --images-only --plan
fv project import --source ORIGINAL.zip --destination NEW_FOLDER --images-only
Set-Location NEW_FOLDER
fv extract all
fv benchmark compare --baseline ORIGINAL.zip
```

Import returns exit 3 for legacy attention; extraction and comparison can also return 3 while successfully retaining attention results. `--images-only` is optional and restricted to import; ordinary full-archive import and normal physical production defaults are unchanged. [Detailed import contract](../guides/LEGACY_IMPORT.md).

This recipe runs saved-file extraction and its native fallbacks. It does **not** run Office conversion or the full processing pipeline's earlier composite/mirrored-image handoff. The result is an extraction benchmark, not full production yield, end-to-end timing or a customer-ready delivery test.

## Actual isolated run

Source: `TextilMuzeum_Floppy_Archive_20260920_115210.zip`, SHA-256 `5a002153f77523fa73766d333e58e11a0edb4016080d8c1e5ce39dd707de0cf1`.

Fresh project: `C:\Users\User\Documents\Codex\2026-09-22\referenced-chatgpt-conversation-this-is-an\Same-Evidence-20261014`. The requested test-workspace root already has a project file, so the import guard refused to nest inside it. The isolated chat workspace was used; that guard was not relaxed.

- Imported **136 original images plus 137 logs**, **198,867,192 bytes** total. Every selected member was CRC-read, copied, synced and independently rehashed.
- Excluded **5,988 old recovered/converted/recovery/report files**. Before new processing, Extracted/Converted/Recovery contained no files. No baseline payload or managed extraction marker seeded the replay.
- Preserved original DD/truncated lengths; did not pad disk 133 or invent a sector map.
- The final comparator independently rechecked **all 273 inputs against ZIP bytes**, plus the import receipt and current files, before and after comparison. All **136 image identities** match; zero different or unknown image comparisons.
- Original archive and original customer-capture projects remain unchanged. No floppy/board access occurred.

| In-scope source payload observation | Count |
| --- | ---: |
| Reference payloads | 1,159 |
| Exact byte/size matches, allowing renamed identical content | 882 |
| Same-path changed payloads | 0 |
| Unmatched reference candidates | 277 |
| Current-only payloads | 37 |
| Confirmed deleted references excluded by default | 167 |
| Recovery-tool reports excluded | 70 |

**76.1%** is the exact-payload match fraction in this particular extraction-only historical-input run. It is not project completion, a normal modern-capture success rate, semantic document completeness, or a certificate that 277 recoverable whole live files were lost. The unmatched set includes **216 DMDE-filesystem-origin entries, 56 signature-carved entries and five ordinary filesystem entries**; unknown/collision-renamed versions stay in scope without guessing whether they were deleted.

Largest unmatched groups: 074 (50), 066 (46), 133 (27), 040/027 (21 each), 121 (13), 122/030 (11 each), 022 (10), 033 (9), 023/058 (8 each). Original logs are missing/unrecognized for 058/059/065/066/074/075. That withholds a trustworthy readable-sector map; it is not proof the corresponding raw image contains no useful bytes. Disks 066/074 alone account for 96 withheld reference matches. Missing-map snapshot-only candidate recovery is a future investigation, not silently enabled by treating unknown sectors as readable.

## Bugs found and fixed

Three old log hash claims disagree with the saved image bytes: **020, 047, 112**. Previously the legacy loader substituted the historical claim for the current image's identity, causing otherwise snapshot-bound extraction selection and comparison to fail.

Legacy attempt identity now uses the measured saved-file hash. Original log claims remain intact and separate; those disks retain attention. Comparison JSON explicitly records `legacy_log_sha256` / `legacy_log_hash_matches_snapshot`, and audits explain the disagreement. A log's claimed OK label is preserved as history, not promoted to current certification.

The contradictory map cannot authorize automatic or expert catalogue-based compositing/FAT repair/native sector recovery. Planning returns CheckEvidence, automatic donor selection excludes it, and explicit recovery commands refuse it. Existing snapshot-bound outputs are preserved and can be content-compared without asserting physical readability or completeness. Hex-case-only SHA representation differences bind the same bytes; a changed source still refuses selection.

## Receipts and verification

Project receipts: `Reports/LegacyImport.json`, `LegacyImportFiles.csv`, `ExtractionSummary.csv`, `EvidenceAudit.json`, and the immutable `BaselineComparison-1791968297792558700-12580.json` / `.csv` snapshot. Full command output is in `Logs/SameEvidenceExtraction*.log` and `SameEvidenceComparisonFinal.*`. Import receipt SHA-256 bound into the final comparison: `3afc10a498990c6ff8bde55a9e813acd1413a42e7501ae7e240585ed67310751`.

Tests cover empty recovered folders, preview/no-write behavior, exclusion inventory, source/selected CRC failures, skipped-payload non-certification, unsafe/colliding excluded paths, stop/source mutation with retained staging, strict flag scoping, and executable comparison refusing changed inputs. Rebinding a modified input's receipt hash still fails the independent ZIP-byte check. Duplicate/incomplete/escaped/forged inventories refuse. Default import and ordinary comparison remain compatible.

Final quality gates passed: formatting, locked all-target checking, strict Clippy, and **776 passing tests / 33 opt-in tests**, with zero failures. The local receipt is `Logs/QualityGateFinal.log`. The release build passed; executable SHA-256: `5ee7cbe93b5d76b7ea8a07b07c5156ce800a36ad3d8ac98cc7c0de22e1b9ce74`. Its images-only import preview also passed without creating the destination project.

The checkpoint keeps **464/479 TODOs**; the existing same-evidence match/exceed requirement stays open. No denominator changes or new boxes. Next work should investigate the demonstrated gaps and a complete same-input processing replay rather than infer parity from a different set of captures.
