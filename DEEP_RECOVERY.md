# Deeper recovery: directories, file tails and Word/RTF text

Native recovery generation **5** adds automatic saved-image recovery paths. Word-text engine **4** also finds supported compound Word content despite a misleading extension. RTF-text engine **2** detects readable RTF1 content and continues around explicitly marked unsupported single-byte symbol glyphs, without guessing their meaning. Both retain the previous conservative mapping/prefix limits and publish separate forensic editions. Normal `fv scan` / `fv process` invokes these when native recovery is needed. No floppy, new policy, DMDE step or recovery decision is required.

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

Both run native recovery; `documents` focuses its output on the additional forensic Word/RTF-text results. Add `--json` for machine-readable counts/paths. **Exit 3 is expected attention**, including successful reuse: the original disk/document remains damaged. Filename extensions are hints, not format proof: eligible partial files with a readable compound/RTF1 signature also get the appropriate metadata checks.

If text survives, the command prints **Open readable edition: ...html**. Open that file in your browser: one page presents the segments in order, with a red block for each missing range. There is no manual text assembly or Office repair step. The page works offline, does not execute macros or fetch pictures/links, and is clearly labeled as a generated recovery edition, not the original DOC. It remains forensic evidence under `Recovery`, not an automatically certified delivery original.

Do not initialize/reset the old project, edit its internal reports, or copy recovery payloads over originals. Source images/maps/hashes must match; changed managed output is preserved and refused rather than overwritten.

## What is saved

| Result | Forensic location | Delivery treatment |
| --- | --- | --- |
| Lost-parent directory tree | `Extracted/NNN/attempt_NNN_native_v5/DirectoryRecovery/cluster_CCCC/...` | Separately marked `Directory-Recovered`; root name/parent/ownership unknown |
| Validated missing-link tail alternatives | Same generation, under `FragmentRecovery` | Separately marked `Fragment-Hypotheses`; historical association unproven |
| Intact signature candidate | Same generation, under `SignatureRecovery` | Existing `Signature-Recovered` treatment |
| Readable pieces of an incomplete file | `Recovery/NNN/attempt_NNN_fragments_v2` | Raw evidence only, not complete documents |
| Word main-text segments, readable HTML and report | `Recovery/NNN/attempt_NNN_word_text_v4` | Forensic evidence/generated presentation only; no routine conversion or original replacement |
| RTF readable-prefix text segments, HTML and stopping report | `Recovery/NNN/attempt_NNN_rtf_text_v2` | Forensic text only; unsupported glyphs are gaps, remaining text length unknown |

Legacy images use `legacy_native_v5` rather than an attempt extraction name. The native report is `Recovery/NNN/attempt_NNN_fat12_v5.json`. Older native generations, fragment-v1 folders, Word-text-v1/v2/v3 and RTF-v1 folders, and forensic deleted-v1 results stay intact. Same-acquisition selection cannot replace an ordinary reachable file solely with reconstructed/hypothetical copies. The archival ZIP includes forensic recovery evidence and generated HTML with warnings; inclusion is not customer-delivery certification.

## Advanced: lost directories

When a parent directory entry is missing, allocated directory bytes may still survive elsewhere. The engine seeks an unclaimed terminated FAT chain, a readable `.` entry pointing to itself, a valid `..` parent hint and surviving plausible child entries. Available FAT copies must agree. Known deleted allocation, shared chains, live-file parent conflicts, invalid anchors and cyclic orphan-parent components are refused.

The wrapper `cluster_CCCC` is **not an original directory name**. Historical live/deleted status can be unknown despite excluding known deleted entries. Nested names and file chains pass the same existing safety/ownership checks; missing directory sectors remain explicit gaps. Children beyond a gap can survive, but hidden entries/names are never invented. Discovery is bounded to 256 corroborated directory candidates; insufficient/ambiguous anchors remain unresolved.

## Advanced: fragmented files with one missing link

Intact fragmented FAT chains were already supported. Generation 5 additionally searches a specific damaged case: a mapped readable file prefix whose next FAT entry is unavailable in every copy, plus a terminated allocated unclaimed suffix of exactly the remaining cluster count. Known deleted/shared/owned allocation and unreadable candidate data are excluded.

Supported format validation must accept a payload starting at the known file beginning and extending into the proposed suffix. A valid header entirely inside the prefix is not relabeled as a missing-link recovery. Actual FAT links, the missing-link cluster and proposed suffix head are recorded separately: **no FAT link is fabricated**. Competing valid tails remain alternatives, not automatically certified originals. RTF structure or a valid CRC/container cannot prove historical association.

Bounds include 64 suffix trials, 8 MiB of candidate input and a shared 128 MiB validation-work budget for this search; signature probe limits also apply. Combined signature/hypothesis publication is capped at 256 payloads / 4 MiB. Search-limit attention explicitly means not exhaustive. Multiple erased boundaries, free-cluster rearrangements and arbitrary fragment permutations are not solved by this feature.

## Advanced: document-aware text salvage

Incomplete Word 97–2007 binary candidates can yield readable main-document text when their required CFB mappings, common Word FIB fields and CLX piece table survive. Engine 4 also dispatches readable compound signatures with other extensions; a compound header alone is insufficient: root Word/table streams and FIB/CLX must validate. A misnamed spreadsheet is not treated as Word. JSON records `content_detection` and HTML explains the mismatch; original names/bytes are not changed. Saved **128**, `vasalók.rtf`, is binary Word and yields **683/939 main-text positions**, with an explicit **256-position gap**. Encryption, unsupported versions, inconsistent bindings and missing required text-mapping metadata are refused. Unreadable acquisition bytes never become decoded text.

Word-text engine **3** adds two narrowly scoped paths. CFB storage objects' unused starting-sector/size fields are retained as raw evidence, not mistaken for stream allocations; root mini-stream and actual stream bounds remain mandatory ([MS-CFB storage entries](https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-cfb/b37413bb-f3ef-4adc-b18e-29bddd62c26e)). When both extension-count bytes are unavailable in a readable Word97-sized common FIB prefix, the engine can salvage main text from the independently readable CLX. It does **not** infer the count, effective version or complete header validity. Partly readable counts, readable contradictory fields, required mapping holes and encrypted files remain refused. Candidate text must lie beyond every supported extension size; that exclusion is a policy bound, not a reconstructed header. JSON records `incomplete_fib` and HTML visibly warns **Incomplete Word header — text-only candidate**. The common-prefix mapping follows [MS-DOC text retrieval](https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/01d5d8c4-cf9c-4ef9-80fd-439e763cfe01); no formatting/container repair is claimed.

Saved full-batch disk **052**, `fejléc_logoval.doc`, yields **1,792 / 1,792 declared main-text positions**, previously refused by engine 2 and first published by engine 3. The source image, old v2/v3 evidence and existing completed delivery remain preserved. Exact readable text extents, metadata, allocation links and unused raw storage fields replay against the image. These are forensic text positions, **not a new intact DOC or a historical-byte parity gain**. Run `fv recovery documents 52` inside the saved project to publish/reuse a separate current-generation edition; exit 3 means attention, not a failed operation.

The normal strict compound-file reader runs first. If it fails, v2 can follow independently readable allocation links and the **complete readable root sibling tree** to `WordDocument` and the FIB-selected `0Table`/`1Table`. Missing nested directory slots are reported and bypassed only when they do not lie on that required root path. Detached entries merely bearing the right name are not accepted. No directory entry, FAT link or stream identity is guessed or rewritten.

This selective path supports 512-byte CFB v3 and 4,096-byte v4 sectors, fragmented regular streams and mini streams. Invalid headers, missing required links/entries, cycles, duplicate root names, known shared allocation and payload/metadata overlap are refused. Unavailable entries cannot establish every historical ownership claim; unrelated malformed allocations remain reported, and `whole_container_verified` stays false. This is stream salvage, **not a reconstructed compound container**. Reports retain the strict refusal, raw root entries, selected sector chains and physical offsets/values for each observed FAT/mini-FAT link.

Each `.txt` is a separate contiguous readable segment. `word-text.json` records its parent, zero-based character-position (`cp_start`/`cp_count`) range, Word stream offset, encoding, source-image byte extents and output hash. Missing/invalid UTF-16 positions are explicit gaps; the program does not join text around them or insert guessed words. UTF-16 positions count code units, so a surrogate pair occupies two positions. Extent `file_offset` indexes the original **encoded segment bytes**, not UTF-8 output bytes.

The `.html` edition is a new, escaped presentation of those same text segments and gaps. Control characters remain literal in `.txt`; non-whitespace controls appear as visible Unicode escape labels in HTML. Original field codes are displayed as text, not executed or interpreted. The report records the edition's path, size and SHA-256; `--json` also returns `readable_edition_paths` and `selectively_mapped_documents`.

This is **text salvage, not a repaired DOC**. Formatting, table structure, pictures, revisions, headers/footnotes, fields and original semantics are not reconstructed/certified. Even zero missing main-text positions does not mean the original container or other stories are complete. Text/editions are excluded from whole-file counts and normal Office conversion. Raw file fragments remain available when parsing is refused.

Bounds: 4 MiB parent/stream size, 64 documents and 500,000 declared main-text positions per run; 200,000 positions / 4,096 segments / 64 MiB strict read work per document; 64 KiB CLX and 16 MiB report ceiling. Selective CFB mapping allows at most 4,096 directory entries / 65,536 allocation visits and inline DIFAT only. Editions are bounded to 4 MiB each / 8 MiB combined; raw segments remain when an edition exceeds its budget. Immutable staging and source/report/text/HTML/inventory verification protect repeat runs. Edited editions are preserved and refused, never overwritten.

The interpretation follows Microsoft's [MS-DOC text retrieval](https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/01d5d8c4-cf9c-4ef9-80fd-439e763cfe01), [FIB structure](https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/9aeaa2e7-4a45-468e-ab13-3f6193eb9394), [compressed text offsets/mapping](https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/aa2e55a2-f4f2-4795-bab5-6d9d7a0ed249) and [CFB directory specification](https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-cfb/a94d7445-c4be-49cd-b6b9-2f4abc663817). The refusal/search budgets are FluxVault policy, not guarantees in those specifications.

## Advanced: damaged RTF readable prefixes

`fv recovery documents 22` also attempts automatic prefix decoding of eligible incomplete RTF files, including readable RTF1 signatures with other extensions. The first unreadable/unmapped file byte is a hard input boundary: later raw fragments remain preserved, but are not joined using guessed group/font state. Unsupported visible control words, unknown/multibyte font encodings, malformed tokens and unsupported UTF-16 surrogate units stop interpretation even earlier. The report records that stopping byte/reason and remaining uninterpreted **file bytes**, not an invented missing-character count.

Engine 2 can continue through a known single-byte symbol-font run (`fcharset2`, no code-page override). It does **not** convert Wingdings/Symbol bytes into guessed letters or icons. Each uninterpreted glyph has its encoded byte, font identity, exact input token interval and decoded UTF-8 boundary in `unsupported_font_gaps`. RTF syntax/font state remains continuously readable; subsequent known-font text can resume without acquisition-hole resynchronization. HTML inserts visible gap panels; separate `text_segments` files retain text on either side. With gaps, `text_path` is null: no silently joined text file is published. `text_sha256` is only an internal ordered-text rollup; segment hashes identify actual files. JSON/CLI report `unsupported_font_glyphs` and `text_segments`. Unknown fonts, explicit unsupported code pages and duplicate font/charset/code-page bindings still refuse interpretation.

The bounded decoder tracks group-local font/Unicode fallback/hidden/deleted state. It supports ANSI Windows-1250/1252 text, supported font charset/code-page bindings, escaped braces/backslashes/hex bytes, signed BMP `\uN` and bounded `\ucN` fallbacks. Font/color/style metadata, pictures/objects, fields, headers/footers/footnotes and ignorable destinations are excluded; binary payloads are skipped only within a known skipped destination and wholly readable input. It executes no Office code, object, field or macro. This is a conservative subset, not a full RTF renderer or complete editable-file reconstruction. A filename alone does not establish a readable RTF1 header.

`rtf-text.json` retains the native incomplete-parent/FAT/missing-range evidence, exact logical prefix length, image extents and SHA-256, decoded-through offset, stop reason and text/HTML hashes. Immutable publication/replay refuses edited reports/text/editions or extra output files. Normal manifest/Office queues gain no whole files/jobs from these derivatives. Committed editions are archived as warned evidence; `.tmp-word-text-*` and `.tmp-rtf-text-*` staging is excluded.

Bounds: 256 KiB readable input per parent; 4 MiB combined input, 64 parents, 500,000 combined UTF-8 text bytes; 200,000 UTF-8 bytes, 128 group levels, 256 fonts and 4,096 glyph gaps per document; 4 MiB per HTML / 8 MiB combined editions, 16 MiB provenance ceiling. Unsupported oversized/malformed mappings yield per-document refusals, not loss of the disk's independently recovered files. Unknown ignorable metadata cannot redefine a known font. Missing/unsupported content is never replaced with a guessed word or replacement character. Group/parser behavior follows [Microsoft RTF syntax](https://learn.microsoft.com/en-us/previous-versions/office/developer/office2000/aa140284(v=office.10)), [reader conventions](https://learn.microsoft.com/en-us/previous-versions/office/developer/office2000/aa140286(v=office.10)) and [font-aware text extraction](https://learn.microsoft.com/en-us/openspecs/exchange_server_protocols/ms-oxrtfex/205e1abf-b794-4fd0-b1e4-5210882233ab). Charset 2 is [SYMBOL_CHARSET](https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-one/64e2db6e-6eeb-443c-9ccf-0f72b37ba411); glyph mappings are deliberately not assumed. Refusal limits are FluxVault policy.

Saved **022**, `nka261056994.rtf`: engine 2 yields **5,318 readable UTF-16 positions** from selected attempt 2, up from engine 1's 330. It reaches file byte **143,867**, stopping on a control token cut by the 143,872-byte readable prefix boundary, not by guessing its delimiter. Attempt 1 yields **3,583 positions** within its 73,728-byte prefix. Unsupported symbol glyphs remain explicit gaps; the rest of the file remains uninterpreted with unknown text length. Engine 1's earlier editions, raw fragments and completed archival snapshots stay intact. Saved 128's `.rtf`-named candidate is correctly refused by the RTF decoder but now independently salvaged through validated binary-Word dispatch.

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
