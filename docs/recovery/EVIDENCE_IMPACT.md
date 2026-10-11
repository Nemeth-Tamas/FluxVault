# What does a bad or disputed sector affect?

These optional diagnostics read saved images only. They do not reread a floppy, extract anything, start tools, change your preferred image, create reports, or interfere with station custody. They are not required during feeding.

## Beginner: see affected files

From the project folder:

```powershell
fv recovery impact all
fv recovery impact 59
```

`all` inspects the **whole saved batch**, one preferred/default native image per discovered disk label. It prints a summary, a disk table, affected known file names and directory/tail warnings. Progress goes to the error stream, so it stays visible without contaminating `--json` output. It never opens a floppy. For exact offsets and LBAs, inspect an individual disk afterward.

```powershell
fv recovery impact all --json
# Optional: save the full report yourself, outside the managed project outputs.
fv recovery impact all --json | Set-Content -Encoding utf8 '..\batch-impact.json'
```

Disks with inconsistent/missing evidence, legacy-only images or raw/partial captures without a completed native image get explicit **REFUSED** rows; other disks still appear. Unsupported/unknown filesystems are **UNKNOWN**, not empty clean disks. Totals include only inspected selected images and count files per disk, not deduplicated customer documents or conversion results. `--attempt` is per-disk only and is refused with `all`.

Run after scanning/background finishing is idle for the most useful result. Each disk is checked consistently, but the entire project is **not an atomic snapshot**. If the numbered image/raw/preference inventory changes, the result warns and exits with attention: rerun when idle. Only discovered saved labels are included; this is not a check that an intended 001–136 range was fed correctly or finished. Batch bounds: 4,096 labels, 100,000 directory entries, 64 MiB of serialized per-disk details; exceeding a bound refuses instead of silently truncating a supposedly clean batch.

The result lists each missing/conflicting sector, its filesystem region, and any known live file that depends on it. A payload dependency gives the **byte offset inside that file**, even for fragmented allocation. Metadata dependencies mean the file's name, layout, directory ancestry or allocation relies on that sector; they do not mean every byte of the file is corrupted.

A sector can be in the boot/reserved area, a particular FAT copy, the root directory, a data cluster, or trailing volume space. If layout cannot be established, the region stays unknown. Missing directory entries and unlocated tails remain explicit: **no known file dependency does not mean the sector is unused or harmless**.

For a same-disk USB/GW disagreement confirmed with `g confirm N`, the observed GW bytes remain present and both editions are preserved. The report shows those sectors as disputed, not missing or repaired. It does not pick an original-content winner or certify either reader.

## Advanced: trace a particular live file

```powershell
fv recovery trace 59 'FOLDER\LETTER.DOC'
fv recovery trace 59 'FOLDER\LETTER.DOC' --attempt 2 --json
fv recovery impact 59 --attempt 2 --json
```

Use the path recorded in the live FAT directory, relative to the floppy root. Slash direction and letter case are ignored; no wildcard, basename guessing or host-file lookup occurs. `--attempt` selects an **Images acquisition attempt**, not a raw capture/decode. Default selection follows the saved preferred image or native attention/bad-sector/newness ranking.

Trace contains:

- The ordered file offsets and exact sector LBAs, clipping the final sector to the recorded file size. Fragmented chains remain in logical file order.
- The observed complete payload's hash, or **no payload hash** when the allocation/content is incomplete or ambiguous.
- Exact mapped unreadable ranges and unmapped-tail byte count. Unlocated tails are never guessed contiguous or padded into a pretend recovered file.
- Directory/name/layout/FAT dependency LBAs and the FAT links/copies supporting the chain.
- Replay-verified offline donor attempts/LBAs and mirrored-FAT copy steps, or GW capture/decode IDs and saved stage settings/policy. An offline donor reference can be followed with `recovery sector N --attempt A --lba L`. Unresolved physical-pass detail stays explicitly unknown; source references are not additional physical reads.

Use `--json` for full inventories, gaps, skipped entries, hashes, confidence and provenance. Human impact output focuses on problems rather than dumping whole-file bytes. The older `recovery sector` hex view now labels `confirmed_cross_reader_conflict` and includes the separate sector-version hashes.

## Scope and safety

This maps **live FAT12 chains** in completed native images with 512-byte sectors, at most 4 MiB and a complete matching acquisition log/map. It can inspect warned standard-layout hypotheses already supported by native recovery, without certifying inferred geometry. A refused/unknown filesystem still yields an impact report with unknown regions, not invented file attribution; a file trace refuses without a usable map/layout.

Deleted entries, signature/fragment candidates, manually imported files, converted documents and legacy bare images are outside this live-chain diagnostic. Their existing recovery/conversion reports remain the relevant evidence. A complete mapped file can still have disputed metadata or payload bytes; document semantics, independent flux CRC and customer completeness are not certified.

Changed source bytes, contradictory metadata/logs, malformed maps, missing referenced evidence, unsafe/device paths and redirected files refuse. Existing immutable offline/GW publication proofs are replayed; selected image/metadata/log/proof snapshots are checked again before returning. One report permits at most 65,536 sector-to-file dependency rows; reaching that ceiling is explicitly flagged as a non-exhaustive listing. Other bounds match the [sector inspector](SECTOR_INSPECTION.md).

Exit codes: **0** means no attention in the inspected command scope; **3** means missing/disputed/unknown/inferred/derived or incomplete evidence; **2** means invalid input or inconsistent/unavailable evidence. None is a whole-disk or customer certificate. Retry inspection later if saved evidence changes while processing publishes it.

For `all`, a per-disk refusal produces **exit 2 with the useful batch report still printed**; attention without refusals is 3. Invalid command/input or an exceeded batch bound produces the usual error instead. Empty projects refuse rather than returning an empty success.
