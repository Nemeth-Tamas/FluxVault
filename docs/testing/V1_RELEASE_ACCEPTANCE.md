# v1 release acceptance — 2026-10-18

The operator approved a frozen release boundary: ship the existing supported **scan → recovery → conversion → reports → ZIP** workflow; keep rare-format/deeper-reconstruction research and extra-drive modes post-v1. This is not universal recovery, DMDE yield equivalence or a claim that every long-term TODO is complete.

## Release contract

| Gate | Evidence |
| --- | --- |
| Source media stays read-only | Existing backend/forbidden-command tests; today's acceptance uses saved captures only. No physical-device probes, reads or writes. |
| Supported physical station and lifecycle | Prior protected WinWord/007-HD/009-DD read/stop/resume/background tests and completed live 136-label USB + GW cohort. See [hardware overlap](HARDWARE_OVERLAP_20261013.md), [single-GW readiness](SINGLE_GW_READINESS.md) and [batch review](BATCH_136_REVIEW.md). |
| Short saved-image finishing | `fv finish` refreshes the shared pipeline, creates safe default sibling delivery storage, permits explicitly warned partial archival and publishes a fresh immutable handoff. Existing strict `finalize` defaults remain unchanged. |
| Safe completed resume | Executable mock test and real 136-image `finish resume`: same archive, no new ZIP and unchanged external-tool audit. Interrupted resume retains saved options and the existing stage/replay guards. |
| Customer entry point | `START_HERE.html` in the ZIP and hash-bound `START_HERE-<archive-edition>.html` beside project.json; HU default, EN supported. VALID report entries must match archived hashes before getting direct PDF/editable links. No scripts, remote assets or executable links. |
| Backward compatibility | v1 independently verifies the prior 1,336,138,591-byte actual customer archive and all eleven bound reports without writing files; a customer page is optional for older archives. |
| Upgrade integrity | Older derived-image creator versions are descriptive metadata, not a replay-identity mismatch. Original metadata serialization/version remain byte-identical; all paths, disk identities, geometry, maps, source hashes and replayed bytes still match exactly. Regression rejects changed identity/hash/version types. |
| Quality gate | `cargo fmt --check`, locked all-target check, strict all-target lint and tests pass: **856 routine tests, zero failures; 33 environment-dependent opt-ins remain excluded**. PowerShell completion engine checks pass against release. |

## Full same-evidence 136-image finishing run

Release 1.0.0, eight requested conversion workers; no floppy inserted or physical media accessed. Existing saved images/logs are the inputs, not copied DMDE recovered payloads.

- Elapsed: **30.4 seconds**, mostly verifying/packaging already-processed results; this is not a physical scanning throughput claim.
- 136 disks audited: **94 verified / 42 attention**.
- **692 successful conversion jobs / one protected-file exception**, zero partial jobs or output retries. Existing completed conversions reused.
- Three derived images reverified/reused; creator-version upgrade preserved original metadata.
- New archive: **186,679,842 bytes**, 4,706 inventoried payloads including the generated start page.
- SHA-256: `13397d0d1ada61a08aa43189a409998ee82355f3bf666fbc06b020d9a40b2977`.
- Independent published verification: eleven bound reports, customer page, checksum, matching ready/history receipts, zero files written.
- All **4,354** pre-existing Images/Extracted/Recovery/Converted artifacts and earlier front-and-center handoff files independently rehashed **unchanged**.
- Page inventory independently checks **136 disk anchors / 1,384 modern document links / 165 folder links / zero broken links**, matching the archived manifest and published page hash. HTML is Hungarian, script-free and has a restrictive offline CSP. Local-file browser preview was blocked by browser policy; no visual-render claim or workaround is made.

The archive is **verified archival evidence WITH ATTENTION**, not customer-certified complete recovery. Separate inferred/snapshot/text candidates remain separate. No unreadable bytes are fabricated.

Local acceptance logs are under the chat workspace's `V1-20261018` directory (`QualityGateFinal.log`, `FinishFinal.json/.log`, `NewHandoffVerify.json`, `OlderHandoffVerify.json`, `FinishResume.json`, `CustomerPageCheck.json`, preservation baselines). Customer data/logs/ZIPs are not checked into git.

## Actual USB + GW customer project refreshed

The original modern `Customer-136-Dual-20261010` project also completed `fv finish` under release 1.0.0 with eight requested workers, using only its saved captures:

- **155.4 seconds** for evidence revalidation, processing and verified archival publication; not physical scan throughput.
- All **136 labels**, **107 verified / 29 attention**; attention was not cleared by packaging.
- **832 successful conversion jobs / zero failed / zero partial / zero retried outputs**, yielding **1,664** linked PDF/editable editions. That is 25 more successful jobs than the earlier 807-job handoff, without another scan.
- **7,293** inventoried payloads, **1,344,319,224-byte** ZIP. SHA-256: `5183f452b4aba04a7d2a8fadd0e928bd707106c2e8159771d8aa012ade050273`.
- Root ZIP: `Customer-136-Dual-20261010_20261018_145212_213.zip`; root page: `START_HERE-5183f452b4aba04a.html`. The customer ZIP contains `START_HERE.html`.
- Independent ready verification passes for ZIP, eleven front reports, customer page/checksum and ownership bindings, with **zero writes**. All 136 anchors and 1,664 direct modern-document links resolve to archived members; zero broken links.
- All **6,798** pre-existing Images/Flux/Extracted/Recovery/Converted files and historical front-and-center handoff files independently rehashed **unchanged**. Older ZIPs remain present.

Additional local receipts: `CustomerFinish.json/.log`, `CustomerHandoffVerify.json`, `CustomerPageInventory.json`, `CustomerExistingArtifactsBefore.csv`, `CustomerPreservation.json`. No captured files or ZIPs are committed/pushed.

## Portable packaging

`scripts/build-release.ps1 -Destination OUTSIDE_CHECKOUT` builds a Windows x64 operator folder/ZIP with both executable names, installer/completion, plain-text manual and guides. Only these allowlisted program/documentation inputs are included; projects, saved captures and customer ZIPs are never collected. Existing release output refuses replacement. The portable installer supports this layout without Rust and does not change PATH unless requested with `-AddToPath`.

Local release ZIPs stay outside git; the source checkpoint/version tag identify the release. Build gates do not silently install software, change profiles/PATH, invoke physical hardware or turn skipped opt-ins into passing tests.

## Remaining scope

The historical TODO remains the wider roadmap. Universal every-boundary power-loss acceptance, broader damaged/rare-format reconstruction, independent adapter write-blocking qualification, exhaustive historical yield comparison and additional-drive modes stay open after this supported v1 release. These are not relabelled as completed by changing the release boundary.
