# Deeper recovery: directories, file tails and Word text

Native recovery generation **5** adds three automatic saved-image paths. Word-text engine **2** additionally maps supported Word streams around unrelated missing compound-directory entries and creates readable offline HTML editions. Normal `fv scan` / `fv process` invokes these when native recovery is needed. No floppy, new policy, DMDE step or recovery decision is required.

## Beginner: use it

From an existing project, with no active scan:

```powershell
fv process
```

This reprocesses saved images, applies the newer recovery engine where eligible, converts supported intact candidates and refreshes reports. It needs the usual 7-Zip/LibreOffice tools. Earlier completed background jobs are not automatically reopened by `processing resume`; use `process` for a new engine.

For one saved disk, without external tools:

```powershell
fv recovery extract 24
fv recovery documents 24
```

Both run native recovery; `documents` focuses its output on the additional forensic Word-text result. Add `--json` for machine-readable counts/paths. **Exit 3 is expected attention**, including successful reuse: the original disk/document remains damaged. No eligible partial `.doc`/`.dot` means no document-text result, not a repaired file.

If text survives, the command prints **Open readable edition: ...html**. Open that file in your browser: one page presents the segments in order, with a red block for each missing range. There is no manual text assembly or Office repair step. The page works offline, does not execute macros or fetch pictures/links, and is clearly labeled as a generated recovery edition, not the original DOC. It remains forensic evidence under `Recovery`, not an automatically certified delivery original.

Do not initialize/reset the old project, edit its internal reports, or copy recovery payloads over originals. Source images/maps/hashes must match; changed managed output is preserved and refused rather than overwritten.

## What is saved

| Result | Forensic location | Delivery treatment |
| --- | --- | --- |
| Lost-parent directory tree | `Extracted/NNN/attempt_NNN_native_v5/DirectoryRecovery/cluster_CCCC/...` | Separately marked `Directory-Recovered`; root name/parent/ownership unknown |
| Validated missing-link tail alternatives | Same generation, under `FragmentRecovery` | Separately marked `Fragment-Hypotheses`; historical association unproven |
| Intact signature candidate | Same generation, under `SignatureRecovery` | Existing `Signature-Recovered` treatment |
| Readable pieces of an incomplete file | `Recovery/NNN/attempt_NNN_fragments_v2` | Raw evidence only, not complete documents |
| Word main-text segments, readable HTML and report | `Recovery/NNN/attempt_NNN_word_text_v2` | Forensic evidence/generated presentation only; no routine conversion or original replacement |

Legacy images use `legacy_native_v5` rather than an attempt extraction name. The native report is `Recovery/NNN/attempt_NNN_fat12_v5.json`. Older native generations, fragment-v1 folders, Word-text-v1 folders and forensic deleted-v1 results stay intact. Same-acquisition selection cannot replace an ordinary reachable file solely with reconstructed/hypothetical copies. The archival ZIP includes forensic recovery evidence and generated HTML with warnings; inclusion is not customer-delivery certification.

## Advanced: lost directories

When a parent directory entry is missing, allocated directory bytes may still survive elsewhere. The engine seeks an unclaimed terminated FAT chain, a readable `.` entry pointing to itself, a valid `..` parent hint and surviving plausible child entries. Available FAT copies must agree. Known deleted allocation, shared chains, live-file parent conflicts, invalid anchors and cyclic orphan-parent components are refused.

The wrapper `cluster_CCCC` is **not an original directory name**. Historical live/deleted status can be unknown despite excluding known deleted entries. Nested names and file chains pass the same existing safety/ownership checks; missing directory sectors remain explicit gaps. Children beyond a gap can survive, but hidden entries/names are never invented. Discovery is bounded to 256 corroborated directory candidates; insufficient/ambiguous anchors remain unresolved.

## Advanced: fragmented files with one missing link

Intact fragmented FAT chains were already supported. Generation 5 additionally searches a specific damaged case: a mapped readable file prefix whose next FAT entry is unavailable in every copy, plus a terminated allocated unclaimed suffix of exactly the remaining cluster count. Known deleted/shared/owned allocation and unreadable candidate data are excluded.

Supported format validation must accept a payload starting at the known file beginning and extending into the proposed suffix. A valid header entirely inside the prefix is not relabeled as a missing-link recovery. Actual FAT links, the missing-link cluster and proposed suffix head are recorded separately: **no FAT link is fabricated**. Competing valid tails remain alternatives, not automatically certified originals. RTF structure or a valid CRC/container cannot prove historical association.

Bounds include 64 suffix trials, 8 MiB of candidate input and a shared 128 MiB validation-work budget for this search; signature probe limits also apply. Combined signature/hypothesis publication is capped at 256 payloads / 4 MiB. Search-limit attention explicitly means not exhaustive. Multiple erased boundaries, free-cluster rearrangements and arbitrary fragment permutations are not solved by this feature.

## Advanced: document-aware text salvage

Incomplete Word 97–2007 binary `.doc`/`.dot` candidates can yield readable main-document text when their required CFB mappings, Word FIB and CLX piece table survive. Encryption, unsupported versions, inconsistent bindings and missing required metadata are refused. Unreadable acquisition bytes never become decoded text.

The normal strict compound-file reader runs first. If it fails, v2 can follow independently readable allocation links and the **complete readable root sibling tree** to `WordDocument` and the FIB-selected `0Table`/`1Table`. Missing nested directory slots are reported and bypassed only when they do not lie on that required root path. Detached entries merely bearing the right name are not accepted. No directory entry, FAT link or stream identity is guessed or rewritten.

This selective path supports 512-byte CFB v3 and 4,096-byte v4 sectors, fragmented regular streams and mini streams. Invalid headers, missing required links/entries, cycles, duplicate root names, known shared allocation and payload/metadata overlap are refused. Unavailable entries cannot establish every historical ownership claim; unrelated malformed allocations remain reported, and `whole_container_verified` stays false. This is stream salvage, **not a reconstructed compound container**. Reports retain the strict refusal, raw root entries, selected sector chains and physical offsets/values for each observed FAT/mini-FAT link.

Each `.txt` is a separate contiguous readable segment. `word-text.json` records its parent, zero-based character-position (`cp_start`/`cp_count`) range, Word stream offset, encoding, source-image byte extents and output hash. Missing/invalid UTF-16 positions are explicit gaps; the program does not join text around them or insert guessed words. UTF-16 positions count code units, so a surrogate pair occupies two positions. Extent `file_offset` indexes the original **encoded segment bytes**, not UTF-8 output bytes.

The `.html` edition is a new, escaped presentation of those same text segments and gaps. Control characters remain literal in `.txt`; non-whitespace controls appear as visible Unicode escape labels in HTML. Original field codes are displayed as text, not executed or interpreted. The report records the edition's path, size and SHA-256; `--json` also returns `readable_edition_paths` and `selectively_mapped_documents`.

This is **text salvage, not a repaired DOC**. Formatting, table structure, pictures, revisions, headers/footnotes, fields and original semantics are not reconstructed/certified. Even zero missing main-text positions does not mean the original container or other stories are complete. Text/editions are excluded from whole-file counts and normal Office conversion. Raw file fragments remain available when parsing is refused.

Bounds: 4 MiB parent/stream size, 64 documents and 500,000 declared main-text positions per run; 200,000 positions / 4,096 segments / 64 MiB strict read work per document; 64 KiB CLX and 16 MiB report ceiling. Selective CFB mapping allows at most 4,096 directory entries / 65,536 allocation visits and inline DIFAT only. Editions are bounded to 4 MiB each / 8 MiB combined; raw segments remain when an edition exceeds its budget. Immutable staging and source/report/text/HTML/inventory verification protect repeat runs. Edited editions are preserved and refused, never overwritten.

The interpretation follows Microsoft's [MS-DOC text retrieval](https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/01d5d8c4-cf9c-4ef9-80fd-439e763cfe01), [FIB structure](https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/9aeaa2e7-4a45-468e-ab13-3f6193eb9394), [compressed text offsets/mapping](https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/aa2e55a2-f4f2-4795-bab5-6d9d7a0ed249) and [CFB directory specification](https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-cfb/a94d7445-c4be-49cd-b6b9-2f4abc663817). The refusal/search budgets are FluxVault policy, not guarantees in those specifications.

## Checked against saved evidence — 2026-10-08

- Isolated customer **021–032** retains all **64 existing payloads** (53 reachable / 11 carved) and nine raw fragments / 449,539 bytes. Original saved image bytes remain unchanged.
- Disk **024**, `BIOFIZ.DOLG.doc`: two UTF-8 segments preserve **1,053 readable main-text positions**, with zero mapped main-text gaps. The parent still has incomplete allocation; this is not a repaired DOC, a new complete-file count or a unique-content yield claim.
- Disk **027**, `LBA2.doc`: Word-text v1 refused the damaged compound directory. V2 independently maps the surviving root Word/table paths around unavailable nested directory slots 4–7 and produces **31 segments / 25,672 readable positions**, with an explicit **256-position gap** out of 25,928 declared positions, plus one offline HTML edition. All decoded text, required metadata and recorded allocation links replay against readable source bytes; no original container repaired.
- Separate isolated customer **001–020**: disk **017**, `ajanlat2.doc`, preserves **3,229 main-text positions** in five UTF-8 segments with zero mapped main-text gaps; its original container is still incomplete. Retained project: `C:\Users\User\Desktop\FluxVault-Test\Native-V5-001-020-20261008-v1`.
- Synthetic tests cover v3/v4, fragmented regular/mini streams, both table selectors, missing nested directory slots and **644 of 900 Word positions** around a 256-position gap. Exact source extents reproduce the text; detached names, required-directory holes, cycles, duplicate names, allocation crosslinks, encryption and changed text/HTML/inventories are refused. HTML escapes untrusted markup and has no active content or remote assets.
- Release `process` on the isolated 021–032 project completes **39/39 Office conversions**, zero errors; all disks remain evidence-attention outcomes. Text salvage adds no ordinary conversion jobs or whole-file counts. The prior 64 payload paths, sizes and hashes match the generation-4 validation exactly.

Final Word-v2 validation projects: `C:\Users\User\Desktop\FluxVault-Test\Word-CFB-021-032-20261008-v1` and `C:\Users\User\Desktop\FluxVault-Test\Word-CFB-001-020-20261008-v1`. Prior native-v5 validation/evidence stays intact; an intermediate v2 experiment under `Native-V5-021-032-20261008-v3` is preserved and is not the final edition checkpoint. No new physical read was required.

Developer replay (read-only, saved projects only):

```powershell
$env:FLUXVAULT_WORD_TEST_PROJECT = 'C:\Users\User\Desktop\FluxVault-Test\Word-CFB-021-032-20261008-v1'
cargo test --lib saved_word_reports_replay_source_extents_and_readable_editions -- --ignored --nocapture
```

Still separate work: erased fragmented/deleted directories, multiple missing allocation boundaries, missing required CFB metadata/structural reconstruction, legacy non-OLE Word, other document formats and genuine editable-document reconstruction. This is a completed bounded engine upgrade, not complete DMDE yield parity or a guarantee that every corrupted document can be repaired.
