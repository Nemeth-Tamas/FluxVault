# Improved saved images flow into file recovery automatically

## Beginner: no extra command to learn

Normal `fv scan` background processing already runs this step. To reprocess an existing project from saved evidence, with no floppy inserted:

```powershell
Set-Location 'C:\path\to\your\project'
fv process
```

If different attempts contain complementary readable sectors, FluxVault builds a separate composite. If readable FAT redundancy can supply a missing FAT sector, it can also reconstruct that sector. The improved image now enters the normal image catalog and is used automatically for native file recovery, manifests, delivery planning, Office conversion, audit and workbook reports. Existing operator-created recovery folders still take precedence.

You do not need to copy files out of `Recovery` or select a composite manually. Earlier acquisition images, forensic extractions and reports remain intact. Repeating processing verifies/reuses the same publication rather than adding another derived attempt. Update the installed `fv` copy after rebuilding; see [installation](TUTORIAL.md#1-install-or-refresh-the-short-command).

## Read the result

An improved catalog entry has status **DERIVED**, never `OK`, even if all missing sectors were supplied. Its attempt number is an image-catalog slot, **not another physical read**. It remains an attention result: file completeness and original custody are not certified by reconstructing an image. A clean physical acquisition stays preferred over a derived result.

`process` reports derived images ready/reused/declined; `--json` exposes `published_recovery_images`, `reused_recovery_images` and `declined_recovery_publications`. The ready count includes reused entries. Unsupported or unverifiable publication is declined with a recorded reason; original-image processing continues. Changed already-catalogued lineage is an integrity error, not an excuse to silently fall back.

```powershell
fv disk show 7 --details
fv recovery queue
fv conversion issues
```

Exit `3` means attention, including explicit derived evidence or declined publication. It is not necessarily an operation failure. Packaging preserves the lineage report and rechecks catalogued derivations; an archival ZIP is not recovery certification, and `finalize` may decline an attention project.

## Advanced: what is verified

The handoff uses original completed acquisitions only, not its own derived outputs as recursive donors. It seals each source image, metadata and authoritative sector log; every generated recovery image/report is sealed too. Before accepting/reusing/exporting a derived attempt it:

- Rechecks source identity, hashes, complete sector maps and bounded geometry.
- Requires all mutually readable source sectors to agree; refuses conflicting captures.
- Replays the exact missing-sector donor choices and records their original attempts.
- Recomputes permissible mirrored-FAT redundancy copies instead of trusting a derived hash alone.
- Requires replayed bytes, unresolved-sector map, final metadata and final log to match.

No missing file bytes are guessed. FAT redundancy is labeled reconstruction, not independent physical corroboration. Matching local controls do not prove authenticity if someone replaces the entire evidence set.

### USB + catalogued GW evidence

Completed GW image acquisitions can participate alongside USB attempts. Before standalone/automatic compositing, native extraction or explicit preferred-image selection uses a GW source, FluxVault now replays the source publication's immutable capture/decode stages. It verifies raw or losslessly packed source hashes, decode byte/map bindings and stage settings, compares every recorded sector origin and requires the exact published image/geometry/unresolved map. Composite publication/reuse also rechecks this lineage through its sealed source metadata. Missing, changed, relabelled, contradictory or oversized proofs refuse; they do not become clean donors through a matching final image hash alone.

This strengthens the supported catalog handoff, **not arbitrary raw/decode promotion or independent MFM/CRC validation**. Existing single-capture/vendor-reported confidence stays visible; DERIVED composites remain attention. A later mutable job is not substituted for an older publication's stages. Verification reads saved captures and can add hashing time; it does not run `gw`, acquire new media or materialize packed sources.

Five new routine mock-chain regressions cover complementary USB/GW gaps through exact payload extraction/audit and repeat reuse, packed retention, historical publication replay, changed raw/decode refusal before publication, semantic proof/map/settings/geometry tampering, missing/oversized proofs and preferred/sector-inspector guards. Saved actual 059/066 catalog lineage also replays with five/four unresolved sectors respectively; their original images, metadata and publication proofs remain unchanged. This is compatibility/integrity evidence, not newly recovered customer content.

Current bounds: at most **16 original source attempts**, complete **512-byte sector images up to 4 MiB**, and complete corroborating acquisition geometry. A DMDE-only set without such geometry is conservatively declined. This is a handoff for catalogued acquisition images, not arbitrary raw captures, unsupported formats or a universal USB/flux donor certification system.

Expert `recovery composite N` / `recovery fat N` still create standalone recovery artifacts. Automatic catalog publication is part of `process` and its scan/background callers. `extract all` alone does not run the composite-planning stage.

## Saved records and restart

| Path | Purpose |
| --- | --- |
| `Recovery/NNN/*` | Original composite/FAT stage images and their sector provenance |
| `Reports/OfflineDerived-NNN-<key>.json` | Immutable source bindings and replay recipe |
| `Images/NNN_attempt_NNN.img` / `.json` | Catalogued DERIVED image and metadata |
| `Logs/NNN_attempt_NNN.log` | DERIVED map/hash log; not a physical acquisition log |
| `Images/NNN_attempt_NNN.partial.json` | Immutable reserved publication slot; excluded from customer packages |
| `Reports/OfflineRecoveryDecisions.json` | Latest decisions, publication attempt/report or declined reason |

A slot is reserved before report/image/log publication; completed metadata is published last. Resume can finish that same immutable intent without reusing its slot for a physical capture. Existing differing or operator-edited output is never overwritten. Temporary/private records are not completed acquisitions. Keep the whole project and do not hand-edit these records.

## Validation checkpoint — 2026-10-07

Routine fixtures cover complementary boot/directory/data gaps through processing and delivery, composite followed by FAT repair with an unresolved file sector, standalone FAT repair, source/report/image/log changes, relabeling refusal, bounded recipes, manual-folder preservation, repeat reuse and archival lineage checks. Five deterministic durable-boundary states exercise reservation/report/image/log/metadata resume; these are checkpoint-state simulations, **not real process-kill or power-loss tests** for this publisher.

On a **new isolated copy** of saved 007 (1.44 MB) and 009 (720 KB), complementary missing sectors were simulated in two attempts. Both derived images matched the original image bytes exactly; all **20 native file payloads** matched their original path/hash/size, with **19 delivery originals** after the existing system-file exclusion. Original saved image hashes were unchanged. This verifies the handoff on real saved filesystem content; it does not claim newly recovered customer files or an increased historical recovery rate.

Retained validation: `C:\Users\User\Desktop\FluxVault-Test\Offline-Derived-HD-DD-20261007-v1`, with `Reports/OfflineDerivedValidation.json`. No physical media is accessed by this validation.

The release CLI subsequently processed that isolated project with real saved-file tools: all **18 Office conversion jobs** succeeded, with zero partial/failed conversions; audit/workbook generation completed and both disks stayed DERIVED attention results. Repeating processing reuses the verified images and bound conversion outputs. An expert archival package independently verified all **118 source members** against their manifest; this is archival integrity, not customer-delivery certification.
