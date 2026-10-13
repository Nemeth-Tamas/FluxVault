# Missing-boot recovery and competing layouts

## Beginner

With a saved project and no active scan:

```powershell
fv process
fv recovery extract 17 --json
```

No extra policy or physical read is required. New native generation **6** preserves older files/reports and uses ordinary selection, conversion and audit rules. Attention remains normal when boot/data bytes are missing or the layout is inferred. Previously completed background jobs are not silently reopened; `process` is the explicit saved-batch upgrade.

When another compatible pass contains readable boot/directory sectors, the existing automatic composite workflow copies those **exact observed sectors**, recording donor attempt/hash/LBA, then analyses the resulting derived image. It never invents absent directory names or boot fields. Complementary-pass regression restores boot LBA 0 and root LBA 19 byte-for-byte, retains both originals and reuses the same composite.

## What generation 6 adds

The previous standard HD/DD fallback is retained as a warned hypothesis. New comparison checks other bounded FAT12 layouts against agreeing readable FATs, actual root entries, terminated size-consistent chains and surviving file contents. It reuses existing PNG/JPEG/BMP/GIF/RTF/ZIP/compound structural validators, requiring coverage of the directory-declared file extent. Unknown formats are not fabricated or promoted into content witnesses.

A broader inferred candidate needs **two distinct, disjoint readable content witnesses** and exactly one such candidate after the complete bounded search. Lower-evidence alternatives stay recorded rather than being called disproven. Equally supported different mappings, unreadable metadata/content, shared chains, contradictory readable BPBs and truncated searches do not get an automatic new winner. A fully populated root can omit an end marker only with those two witnesses; opaque/deleted-only regions cannot establish it.

These are **inferred layouts, not reconstructed boot bytes or unrestricted original-layout certificates**. Files remain attention/evidence: matching container structure cannot prove historical names/ownership/semantics. Root records without surviving source evidence remain missing. No bytes are guessed into customer images.

## Advanced evidence and bounds

Native JSON `analysis.layout_evidence.competition` records the saved image SHA-256, excluded LBAs, exact tested count, candidates, each content witness's root offset/payload SHA/source LBAs/validator, selected bounded preference and completion/resource ceilings. Failed native analysis retains a separate `layout_competition` report where image size/map permits comparison. A preference there is a structural observation, not authority to override a readable BPB. Old reports lacking these additive fields remain readable.

The family uses two fully readable identical FATs, reserved sectors **1–32**, sectors per FAT **1–32**, root entries **16–1024** in sector-aligned multiples of 16, and sectors per cluster **1/2/4/8/16/32/64**. It is capped at **8,192 trials**, **512 retained candidates** and **128 MiB charged payload-validation work**. Bounds/used work are explicit; cancellation is respected. Partitioned, single-FAT, other sector-size or out-of-family layouts remain unsupported/unknown. No global uniqueness or customer certification is claimed. Layout arithmetic follows the [Microsoft FAT specification](https://www.scs.stanford.edu/~zyedidia/docs/_other/fat.pdf); these search bounds are FluxVault policy.

## Verified saved-image gain — 2026-10-13

Boot loss is simulated **in memory only** on saved pilot disks 005/007/009/012/017. The originals are rehashed/checked unchanged. Results retain **3/19/1/11/15** complete payloads respectively: **49 identical names, hashes, sizes and data-LBA mappings**. The known 017 incomplete document remains excluded from intact-file counts.

017's populated root previously failed because it lacked an end marker. The new comparison finds one supported layout in 343 tested layouts, with **14 structurally verified document witnesses**. This is recovery capability under simulated boot loss—not 15 newly discovered customer files or a claim that the actual captured boot sector is bad.

Ten new comparison/composite regressions cover custom roots, full-root boundaries, ambiguous content mappings, excluded bytes, contradictory readable boots, malformed inputs/maps, old evidence, every resource ceiling, invalid containers/shared chains and exact cross-pass metadata donors. A separate native integration test verifies managed publication, source hash/map/proof, exact two-file output, attention and immutable reuse. Prior-generation/report and full production tests remain part of the routine gate.

Saved regression:

```powershell
$env:FLUXVAULT_FAT12_TEST_PROJECT = 'C:\Users\User\Desktop\FluxVault-Test\Customer-020-Pilot-20261006'
cargo test --locked --lib real_saved_pilot_payloads -- --ignored --nocapture
```

Full gate: `C:\Users\User\Desktop\FluxVault-Test\Layout-Checkpoint-20261013-0af9df49\QualityGate.log` — **766 routine tests pass**, 33 environment-dependent cases opt-in; formatting, locked all-target checking and strict Clippy pass. Media was not accessed for this checkpoint. Historical DMDE yield and independent hardware acceptance remain separate.

Release build SHA-256: `A6FF2BB6DC6ECD748F0809F6736E9A71106FBFD2CA6071A72B214A89E7944FFD`. The later manual-folder regression assertion also verifies no generation-6 child appeared in operator recovery; that targeted rerun and final strict lint pass.
