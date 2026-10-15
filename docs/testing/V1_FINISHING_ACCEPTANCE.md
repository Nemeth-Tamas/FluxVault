# V1 finishing and conversion restart — 2026-10-15

This checkpoint tests the complete **saved-image** finishing chain on the uncontaminated 136-image replay, plus conversion publication interruption and existing delivery verification. It is not another physical scan or a fresh hardware-throughput measurement.

## What changed

Two v1-finishing gaps are addressed:

1. **Office output publication survives loss of the batch cursor.** Before this change, a completed DOCX/PDF could appear at its delivery path before `ConversionState.json` was saved at batch end. After a forced exit, a valid-looking file without that binding correctly refused reuse, but could not automatically resume. New synced per-output receipts seal the exact project/source/path/filter/output hash before final publication. Restart replays the sealed payload without another Office invocation; unsealed work is never adopted.
2. **A completed handoff has a read-only check.** `fv package verify` checks the root ZIP, owned ready receipt, archive-bound report generation, root report copies and checksum without refreshing processing or repairing files. Exit 0 means snapshot integrity, not complete recovery or customer certification. [Operator instructions](../guides/FINALIZATION.md#check-an-existing-handoff-without-changing-it).

LibreOffice now receives a private, hash-checked source copy. Publication uses independent, synced copies and non-replacing hard-link promotion on supported workstation filesystems. Edited sources/targets/receipts/payloads, changed settings, conflicting sealed outputs and foreign project bindings refuse automatic reuse. Working private payloads remain in `Reports/.fluxvault-conversion-output`, cost extra storage and are excluded from delivery archives. Resource admission accounts for the extra copies, including replay of a missing target. The source and individual output limit is 128 MiB.

## Real 136-image run

Project: `C:\Users\User\Documents\Codex\2026-09-22\referenced-chatgpt-conversation-this-is-an\Same-Evidence-20261014`.

This is the existing [images/logs-only import](SAME_EVIDENCE_REPLAY.md): no old recovered or converted files seeded it. Source ZIP SHA-256 remains `5a002153f77523fa73766d333e58e11a0edb4016080d8c1e5ce39dd707de0cf1`. Physical drives were not used.

The real 7-Zip/LibreOffice `process --conversion-workers 12` run completed with attention, not an operation failure:

| Result | Measured count |
| --- | ---: |
| Disk labels audited | 136 |
| Evidence verified / attention | 94 / 42 |
| Disks with extraction | 121 |
| Automatically published mirrored-FAT DERIVED images | 3 |
| Office source jobs | 665 |
| Jobs with both modern/PDF outputs successful | 664 |
| Failed / partial / timed-out conversion jobs | 1 / 0 / 0 |

The failed item is disk 136's `SignatureRecovery/carved_000a0a00_fb1217b03c17.xls`: LibreOffice reports that the source cannot be loaded. It remains a failed carved candidate, not a fabricated repaired spreadsheet. The first full conversion run used the prior release; the subsequent finalization used the new debug implementation, revalidated/reused completed bindings and retried the failed item.

`finalize --allow-attention --conversion-workers 12` then completed in **257.164 seconds**, including **79.398 seconds** of package building/verification. It reused all three derived-image publications. It created an HU-default verified archival ZIP with **4,435 source entries / 497,764,039 source bytes**, plus the package README/manifest sidecars:

- ZIP: `Same-Evidence-20261014_20261015_133325_837.zip`
- ZIP bytes: **183,468,381**
- SHA-256: `7c686266ab701406e31d028cf936ec6118e4d8f864e5a6eb840be81ec8d9c71e`
- Original destination: sibling `V1-20261015-Delivery` in the chat workspace.
- Ready ZIP/checksum and **11 archive-bound final reports** published beside `project.json`.
- `package verify` passed; exit 0, zero files written, no tools/media. Finalization itself correctly returned **3** for `verified_archival_zip_with_attention`; certification remains false.

The same read-only verification passed on the existing modern-capture project `C:\Users\User\Desktop\FluxVault-Test\Customer-136-Dual-20261010`: its **1,336,138,591-byte** root ZIP and 11 front reports still match archived edition SHA-256 `e674184adb051b8cdc45303039356691833190d6d9983a64671e4f6b12437858`. That old edition was not refreshed or repackaged.

## Same-input comparison after full processing

The comparator independently reverified all **273 original image/log inputs / 198,867,192 bytes** against the old ZIP after processing and finalization. The import receipt SHA-256 remains `3afc10a498990c6ff8bde55a9e813acd1413a42e7501ae7e240585ed67310751`.

Ordinary payload comparison is unchanged: **882/1,159 exact**, zero changed same-path payloads, 277 unmatched and 37 current-only. The current preferred-image snapshot candidate scope has **136 candidates / 125 additional exact matches**, or **1,007/1,159** matching available payloads when those explicitly unverified copies are counted separately. This is not verified whole-file recovery or script parity.

The preceding extraction-only checkpoint counted one additional known-hole candidate on 122. Full processing now prefers its mirrored-FAT DERIVED image, so that original-image-bound candidate is outside the comparator's current preferred-image scope. Its original source, immutable candidate generation and earlier comparison remain preserved; it is not a lost file or a newly readable sector. Old aggregate figures must not be mixed silently with the newer selection scope.

## Interruption and refusal tests

- Actual executable forced-exit test publishes the modern output, interrupts during the PDF host, confirms no aggregate conversion state exists, then restarts. The modern output is reused byte-for-byte with exactly one modern host invocation overall; PDF completes on resume. Mock Office only, no media.
- Library cutpoints cover payload-only, sealed-before-publish and published-before-batch-state exits, plus cancellation immediately after sealing.
- Separate cases cover private input identity, source edits before/after sealing, output/payload/receipt tampering, differing settings, oversized/foreign receipts, competing sealed payloads, late operator collisions and independent recovery/output copies.
- Published-handoff tests cover repeated no-write checks, a moved/missing outside original, changed/missing root artifacts, forged report omissions, raw-only editions, busy ownership, mid-check changes and strict CLI flag scoping. The complete executable production fixture also checks that verification changes neither run control nor project metadata/ready pointer/tool audit.

These are bounded process-exit/publication guarantees on the tested filesystem, not universal storage-controller/power-loss qualification or every possible pipeline boundary. The broader TODO items stay open; no checklist denominator or recovery-rate claim changes.

The complete gate passes **825 routine tests**, zero failures, with **33 environment-dependent tests** left explicit opt-ins. Format checking, locked all-target checking and strict Clippy pass. Actual PowerShell 7 completion acceptance also passes separately, including `package verify` and its restricted flags.

The optimized release build passes. Its `package verify` rechecks the replay's complete published handoff, and its real-Office fixture invocation reuses both sealed outputs with no reconversion. Release executable SHA-256: `d59f6fc755bf517b00ddf977c8457e478e2d535ecc2a9c3f57f80504b790a031`.

## Local receipts

Chat-workspace receipts:

- `Same-Evidence-20261014/Logs/V1FullProcess-20261015.json` and stderr log.
- `V1Finalize-20261015.json` and stderr log; immutable project finalization receipts retain phase results.
- `V1ReplayDeliveryVerification-20261015.json` and `V1OriginalDeliveryVerification-20261015.json`.
- `Same-Evidence-20261014/Reports/BaselineComparison-1792064237477066100-22096.json` and CSV; CLI wrapper `V1BaselineAfterProcess-20261015.json`.
- `V1RealOffice-20261015.json` / `V1RealOfficeReuse-20261015.json`: a fresh real RTF becomes DOCX and PDF; the second call reuses both sealed outputs with no additional Office invocation.
- `V1QualityGateFinal-20261015.log`: format, locked all-target check, strict lint and routine tests.
- `V1ReleaseDeliveryVerification-20261015.json` and `V1ReleaseOfficeReuse-20261015.json`: final optimized executable checks.

Keep both working projects and their private receipts. No ZIP belongs in Git.
