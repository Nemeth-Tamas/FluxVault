# FluxVault progress — 2026-10-06 offline preparation

The routine operator path is `fv init` -> `fv scan` -> swap/confirm until finished. New scans identify supported IBM formats and losslessly pack verified raw captures in the background. Existing projects retain their saved settings. This checkpoint uses saved captures only; the next hardware session should smoke-test the new defaults before scaling up.

## How close?

| Measure | Current checkpoint | Meaning |
| --- | --- | --- |
| Routine replacement of the seven scripts in `G.zip` | approximately 85% | Acquisition, extraction eligibility/manual preservation, manifests, bounded conversion, audit/workbook and verified archives are implemented. This is engineering coverage, not proven equality of all historical reports/files. |
| Single-Greaseweazle 136-disk benchmark readiness | approximately 90% | The real 20-disk chain passed; numbered/Enter swaps, limits, resume, telemetry, standard format handling and packed storage exist. New live defaults, nonstandard exceptions and long-run acceptance still need validation. |
| Whole TODO list | 285 / 429 = 66.4% checked | Literal checkbox count, including nested checkpoints, umbrella tasks and historical groundwork; not a weighted product-completeness score. |
| Fully autonomous recovery/product ambition | approximately 60% | Routine scans and saved-file processing work. Severe filesystem damage/carving, automatic ambiguous-format continuation, continuous downstream scheduling and two-station custody/resource control are major remaining work. |

These estimates do **not** mean 85% of customer files are recovered or that 90% of damaged disks are solvable. No matching-yield percentage is supportable until the 136-image/1,667-file script+DMDE baseline is compared. Script compatibility is distinct from automating the manual recovery the scripts deliberately delegated to DMDE.

## This batch

- Conservative IBM 1.44 MB/720 KB format discovery from immutable whole-disk flux, with saved candidate hashes/sector maps/decision reports. Explicit fixed formats/maps remain available. Ambiguity saves evidence and stops custody rather than guessing.
- Persist selected format and automatic/fixed mode through recovery restart. Correctly distinguish per-profile decode attempt numbers, use the selected profile for comparisons/default re-decodes, and record actual format in telemetry.
- Convert-only offline backend: saved-capture identify/decode cannot query the board, even for host version discovery. The installed distribution's bounded local `VERSION` file is used when available; missing version stays unknown.
- One background lossless packer, durable task files, offline `storage resume`, capture ownership locks, atomic no-overwrite publication, verified decompression, explicit/managed retention, capacity checks, isolated materialization and archival export of packed evidence.
- Expanded mock/unit/executable coverage for automatic DD, restart/absent-board reuse, default packing, logical hash preservation, damaged archives, ownership, publication restart and export.
- Updated CLI help, cheat sheet, policy explanation and fresh 136-disk runbook. Original 20-disk runbook is clearly historical; do not rerun its `init` block.

## Real saved-evidence checks

Verification: 209 automated tests passed (8 environment-dependent tests remain ignored), `cargo fmt`/all-target checking are clean, and the release executable was rebuilt. Final suites were rerun after the transient harness failure noted below.

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

1. Short live 007/009 and damaged-disk checks on the new auto/packed defaults, then a fresh numbered benchmark folder.
2. Continue past ambiguous/nonstandard formats with explicit raw-only exception records; do not fabricate image geometry or require journal editing.
3. Compare saved pilot payloads against reachable versus deleted/orphaned/carved baseline content; implement evidence-bounded recovery for the actual yield gaps.
4. Coalesced background extraction/conversion/audit with shared project ownership and acquisition-priority resource limits.
5. Forced interruption/disk-full/cross-process storage soak tests and cleanup of abandoned private temporaries. One concurrent Windows test-harness fast-fail was not reproduced on subsequent complete reruns; it remains an investigation item, not a claimed fix.
