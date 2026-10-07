# Deleted candidates and readable fragments

The everyday workflow remains **`fv scan`**. Incomplete live files now automatically retain their independently readable bytes as separate forensic fragments. Known deleted entries remain excluded from normal extraction and delivery; searching them requires an explicit command.

## Optional deleted recovery

From an existing project folder:

```powershell
fv recovery extract 59 --include-deleted
```

This uses the selected best **saved image** and its completed acquisition map. It needs no floppy, Greaseweazle, 7-Zip or LibreOffice. It returns **3 (attention)**, including successful verified reuse. Add `--json` for structured output. It searches deleted entries only; it does not replace or refresh existing live extraction, conversion or delivery. Run ordinary `fv process` for those.

Outputs are separate:

```text
Recovery/059/attempt_001_deleted_v1/
  DeletedRecovery/deleted_ENTRYOFFSET.ext
  DeletedRecovery/carved_OFFSET_HASH.ext
  .fluxvault-fat12.json
  .fluxvault-inventory.json
  .fluxvault-extraction.json
Recovery/059/attempt_001_deleted_v1.json
```

Actual attempt numbers follow the saved image. These folders never participate in normal `Extracted` selection or `Converted` delivery planning. A full archival package can preserve them under **Recovery**, explicitly as forensic evidence, not ordinary live files.

Two bounded approaches are supported:

- **Surviving terminated FAT chain:** require a size-consistent chain, agreeing readable FAT copies, readable data, no reachable live ownership, no overlapping deleted claims and no external/shared incoming link. Export acquisition-reported readable bytes under a reconstructed offset name. The recorded extension is only a hint; surviving allocation does not establish historical content authenticity.
- **Erased allocation:** a fully readable, all-free contiguous span of the recorded size can be a hypothesis. Only independently validated PNG/JPEG/BMP/GIF, ZIP, RTF or OLE candidates are exported from it. The report retains the hypothesis and deleted-entry association; this is not a claim that the entire historical file was recovered. No unsupported arbitrary data is called an intact deleted file.

The original first short-name character and deleted VFAT sequence numbers are lost; they are **not guessed**. Raw directory entries, parent-directory context, metadata/FAT evidence, exact source extents, hashes, rejected signatures and skipped reasons remain in the report. Ambiguous ownership, reallocation, conflicting FATs, loops, unreadable bytes and insufficient layouts are refused. Deleted directories, fragmented files whose FAT chain was erased and arbitrary document repair remain unresolved.

Deletion recovery is **OFF by default**, including new-project policies. `scan`, `process` and `extract all` do not accept this flag. For `benchmark compare --include-deleted`, the same flag still expands **comparison scope only** and does not run recovery or include these forensic outputs in normal payload scoring.

## Automatic partial-file evidence

Ordinary native recovery now preserves raw readable runs of incomplete live files automatically:

```powershell
fv recovery extract 27
# Or the usual whole-project processing:
fv process
```

They are stored separately:

```text
Recovery/027/attempt_001_fragments_v1/
  fragment_ENTRYOFFSET_FILEOFFSET_HASH.bin
  fragments.json
```

Each fragment records its parent file, **logical parent byte offset**, SHA-256 and exact physical source extents. The report contains the parent allocation/FAT/name provenance, precise unreadable ranges and unmapped tail lengths. Fragment-relative offsets start at zero; the separate parent offset identifies where those bytes belonged.

- A missing sector splits a run. It is never filled or skipped over inside one fragment.
- Known fragmented allocation follows logical file order; physical extents remain explicit.
- Final-sector slack beyond the declared file size is excluded.
- Crosslinked/ambiguous ownership is not exported.
- `.bin` fragments are **not complete documents**, never enter automatic Office conversion or normal live-file inventories, and never increase complete-file counts.
- Unknown layouts and missing directory entries cannot supply invented parent associations.

Limits are 4 MiB total fragment bytes, 4,096 fragments and 16 MiB report size per generation. Full FAT provenance is retained once per parent rather than copied into every fragment. An all-unreadable or ambiguous parent still has a report but can have zero exported fragments. No missing text is guessed.

Source/map binding, the exact report, payload bytes and directory membership are checked on reuse. Edits, missing/extra files or changed sources cause refusal; they are preserved, not overwritten. Publication uses exclusive per-disk ownership and new staging directories. Abandoned private staging is retained for diagnosis and excluded from archival packages.

## Measured saved-capture results — 2026-10-07

On an isolated copy of customer **021–032**, the existing 64 recovered payloads (53 reachable files / 11 validated signature candidates) remain unchanged. New raw partial evidence:

| Disk | Fragments | Readable bytes retained |
| --- | ---: | ---: |
| 022 | 3 | 261,123 |
| 024 | 1 | 24,064 |
| 027 | 5 | 164,352 |
| Total | 9 | 449,539 |

These are additional preserved fragment artifacts, **not nine recovered documents**. Some bytes also occur in independently recovered embedded/signature candidates; totals are not unique customer-file yield. Results are retained in `C:\Users\User\Desktop\FluxVault-Test\Native-Fragments-021-032-20261007-v1`.

On isolated **053–064**, deleted analysis examined **189 entries**, rejected 183 allocation/ownership candidates and tested six contiguous hypotheses. **Zero deleted payloads passed export checks**. The rejected compound candidate crossed the mapped readable extent; a bitmap candidate had an invalid envelope. This validates safe refusal and unchanged live results, not historical deleted-file recovery parity. Results are retained in `C:\Users\User\Desktop\FluxVault-Test\Deleted-Recovery-053-064-20261007-v1`.

Both saved-image regressions verify original source bytes before/after. Synthetic fixtures independently demonstrate surviving fragmented deleted chains and validated free-contiguous candidates, as well as refusal of live reuse, conflicting/overlapping allocation, bad sectors, unsafe names, output/report edits and inappropriate CLI flags.
