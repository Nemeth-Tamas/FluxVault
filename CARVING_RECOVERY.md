# Automatic saved-image carving

Native recovery generation **5** retains generation 4's validated carving and adds [lost-directory / missing-link tail recovery and forensic Word text](DEEP_RECOVERY.md). It salvages validated candidates from allocated orphan chains, readable parts of damaged files, and images without a usable filesystem layout. Normal `fv scan`, extraction and `fv process` use this fallback automatically. No new recovery policy or swap-time decision is required.

To reprocess an existing project without inserting disks:

```powershell
fv process
```

For one saved disk, without Greaseweazle, 7-Zip or LibreOffice:

```powershell
fv recovery extract 24
```

This returns **3 (attention)** even when recovery succeeds. Conversion is part of `process`; standalone recovery does not launch Office tools. Completed background jobs are not reopened merely by `processing resume`; use `process` to apply a newer recovery engine.

## What gets searched

- With a readable FAT layout: follow terminated, allocated chains not claimed by reachable live entries or identified deleted entries. Consult every readable FAT copy and refuse disagreements, cycles and shared tails. Scan in **logical cluster order**, so fragmented chains can produce valid candidates. Free clusters and known deleted chains are excluded by default.
- For a damaged live file: scan only independently readable, allocation-mapped runs, clipped to its declared size. Never join around a missing sector or unmapped tail, never scan its final-sector slack as file content, and never export the incomplete parent as complete. A recovered embedded object records its parent and source extents.
- Without a usable layout: scan contiguous readable physical regions. The report explicitly states that original live/deleted ownership is **unknown**. This is not an opt-in deleted-directory reconstruction feature and cannot establish deletion status.

Unclaimed allocation alone cannot prove a historical file was live rather than deleted. Names remain reconstructed and candidates remain separately labeled. A raw fallback with no validated candidate still saves an immutable zero-file result and the filesystem failure reason; it does not pretend the disk was recovered.

## Validation before export

| Candidate | Required checks |
| --- | --- |
| JPEG / PNG / BMP | Complete format envelope and bounded pixel decode; every PNG chunk CRC checked |
| GIF | Complete block envelope and all animation frames decoded, within limits |
| ZIP | Complete readable archive, safe unique member identities, bounded decompression and member CRC verification; no members extracted/executed during validation |
| RTF | Version/control delimiter, balanced groups, escaped controls and exact binary lengths; text/render semantics are not certified |
| OLE compound document | Strict directory/FAT/mini-FAT parsing and all streams read; recognizable Word/Excel stream headers select `.doc`/`.xls`, otherwise `.ole` |

OLE export reconstructs the **minimal allocated container extent**. Original length and trailing free-sector padding may be unknown, so a usable document can differ byte-for-byte from an archived original. Structural checks are not a guarantee that every Word/Excel object or every original sentence is intact. Supported Office candidates then enter the existing automatic DOCX/PDF conversion pipeline; failed conversions remain explicit attention results.

The parser references the [Microsoft compound-file specification](https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-cfb/05060311-bfce-4b12-874d-71fd4ce63aea) and [PNG specification](https://www.w3.org/TR/png-3/). Pixel and compound-stream validation use bounded read-only library readers.

## Evidence and folders

Generation 5 publishes separately under `Extracted/NNN/attempt_NNN_native_v5` or `legacy_native_v5`. Older managed generations and operator recovery are preserved. Signature candidates use `SignatureRecovery/carved_OFFSET_HASH.ext`; delivery mirrors use **`Signature-Recovered`**, not invented original filenames. Conversion maps and baseline comparisons keep their signature-recovered origin rather than relabeling them ordinary FAT files.

The immutable native report is `Recovery/NNN/attempt_NNN_fat12_v5.json`, also bound by hash to the managed inventory. It includes:

- Source image SHA-256 and acquisition-reported bad LBAs.
- Normal FAT analysis, if possible; otherwise `analysis: null` and `filesystem_error`.
- Per-candidate SHA-256, validation method, allocation scope and precise logical-file/source-byte extents. Fragmented candidates retain their supporting FAT links and metadata LBAs.
- Parent-file association for embedded candidates, rejected signatures, deduplicated payloads, allocation exceptions and work-limit status.
- Existing partial-parent byte-hole ranges and unmapped tails. Missing bytes are not filled or guessed; file/customer completeness remains uncertified.

Reuse rechecks source/map binding and inventory/report hashes. A changed source, missing acquisition evidence, tampered output or competing disk owner is refused. Candidate names and raw offsets do not upgrade single-capture sector confidence.

Work limits: 4 MiB sector images; at most 1,024 signature probes / 256 exported candidates / 4 MiB exported candidate bytes; aggregate validation work bounded to 128 MiB. Individual decompressed archives/compound streams are capped at 32 MiB; pixels at 8,192 per dimension and 64 MiB allocation; GIFs at 64 frames. Hitting a limit is reported, never reported as exhaustive recovery.

## Actual customer-capture result — 2026-10-07

On an isolated copy of saved customer 021–032:

| Disk | Additional candidates |
| --- | --- |
| 023 | Four JPEGs from an allocated orphan chain |
| 024 | Two Word containers: one from the readable prefix of incomplete `BIOFIZ.DOLG.doc`, one from an orphan chain |
| 027 | One JPEG and four PNG objects from readable parts of incomplete `LBA2.doc` |

All 11 candidates retain exact source extents and hashes. Both 024 documents converted successfully to DOCX and PDF; their DOCX text contains 1,018 and 2,296 characters respectively. All **39/39** Office candidates in the isolated project converted successfully, versus 37 candidates before carving.

The historical archive comparison remains **38/89 byte-identical payloads, 51 missing, zero changed**; current-only payloads increase from 13 to 24. Newly usable reconstructed containers/embedded objects are not falsely counted as identical historical originals. Original customer projects/images/captures and the baseline ZIP remain unchanged by this isolated validation.

Results: `C:\Users\User\Desktop\FluxVault-Test\Native-Carving-021-032-20261007-v4-CLI`. `Reports/NativeCarvingValidation.json`, native per-disk reports, `ConversionSummary.csv`, `DeliveryPathMap.csv` and immutable baseline snapshots explain the result.

Automatic raw partial-fragment preservation and explicit forensic-only deleted recovery are implemented; see [commands and measured results](DELETED_AND_FRAGMENT_RECOVERY.md). Generation 5 also supports bounded evidenced lost-directory reconstruction, one missing-link tail hypotheses and mapped Word main-text salvage; see [checks and results](DEEP_RECOVERY.md). Still open: pre-OLE legacy Word/PDF/other signatures, erased/multiple-boundary fragmented allocation, damaged compound metadata and complete historical recovery-yield parity. The program does not brute-force plausible text and call it authentic.
