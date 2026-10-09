# Inspect a saved sector

This is an optional troubleshooting tool, not a step needed during scanning. Work in a project folder; it reads workstation images only.

```powershell
fv recovery sector 7 --lba 0
fv recovery sector 59 --lba 24 --sectors 2
fv recovery sector 59 --lba 24 --attempt 2 --json
```

Disk numbers are the labels recorded during scanning. **LBA starts at 0.** CHS cylinders/heads start at 0, but the CHS sector starts at 1. `--attempt` selects a completed native image acquisition, not a raw capture or decode. Without it, attention/bad-sector count/newness chooses the best native attempt. This does not change the project's selected disk or preferred extraction.

The output includes image/metadata/log paths, the image hash, sector hashes, hex bytes and printable ASCII (`.` replaces nonprintable bytes). Each row shows its absolute image-byte offset. One invocation shows 1–8 sectors, never the whole disk by accident.

## What the labels mean

| Sector status | Meaning |
| --- | --- |
| `readable_saved_evidence` | A completed acquisition log agrees with the saved hash, geometry and map; this sector is not recorded missing. |
| `unreadable_or_conflicting` | The recorded bad map includes it. Bytes may be zero-filled placeholders and must not be interpreted as recovered content. |
| `unknown` | A completed recognized log does not establish a readable map. Matching image bytes alone are insufficient. |
| `derived` | Sector belongs to an offline reconstruction. Exact saved-copy provenance replays, but it is not an independent physical read. |

Composite/FAT-derived images include the original donor attempt/LBA/hash and mirrored-FAT copy steps, after verifying the sealed recipe. GW-derived images can include the **recorded** capture/decode confidence entry after checking its provenance hash and image/disk/sector binding. That record is not an independent raw-flux replay or proof that the operator inserted the labeled disk.

## Exit codes and boundaries

- **0:** the selected range has readable saved evidence. A different sector on the same disk may still be bad.
- **3:** the selected range needs attention (missing/conflicting, unknown or derived).
- **2:** invalid arguments, unavailable attempt, changed bytes/control records, unsafe paths or contradictory evidence.

Only completed native `Images/NNN_attempt_NNN.json` acquisitions are supported. Legacy bare images/DMDE logs are still usable by existing recovery workflows, but not this diagnostic. A recorded nonempty log path must resolve to an existing regular file; unavailable referenced evidence is an error, not an implicit clean map.

Images are limited to 4 MiB; metadata to 1 MiB; logs/provenance to 8 MiB; one disk to 256 attempts, and the Images inventory to 100,000 entries. Invalid/duplicate maps, foreign paths, floppy/device aliases and reparse/symlink files are refused. The inspector verifies a bounded image snapshot and rechecks image/selected metadata/log/provenance snapshots before returning. Run it while saved processing is idle if concurrent changes are reported. It never edits bytes, creates reports, launches tools, materializes captures or changes project state.
