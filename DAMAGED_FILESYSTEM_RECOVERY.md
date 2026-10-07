# Damaged-filesystem recovery

Native recovery gets past some missing boot sectors and now automatically salvages validated signature candidates from orphan chains and damaged-file regions. It runs on saved images, never the inserted floppy. See [automatic carving](CARVING_RECOVERY.md) for generation 4 and its measured customer-capture results; this is not complete DMDE replacement.

## Use it

Normal `fv scan` / `fv process` already tries native recovery for partial acquisitions and failed/empty 7-Zip extraction. No new policy or routine operator step is needed.

To explicitly reprocess a saved disk, including one with an older native extraction:

```powershell
fv recovery extract 9
```

Use `fluxvault` instead of `fv` if you have not installed the short launcher. The command needs neither Greaseweazle nor 7-Zip/LibreOffice. It returns **3 (attention)** even when complete files are recovered. `--json` includes `layout_method`, `layout_warning`, the source hash and the report location. Human output keeps the warning visible on reuse as well.

New results live in `Extracted/NNN/attempt_NNN_native_v4` (or `legacy_native_v4`) and `Recovery/NNN/attempt_NNN_fat12_v4.json`. Older generations/manual folders are preserved, not rewritten. A changed source, incomplete acquisition map or altered managed inventory still blocks recovery. If no layout can be established, generation 4 records `analysis: null`, the filesystem error, and its bounded readable-region carving results instead of inventing geometry.

## What missing-boot inference means

The normal parser uses the readable BPB. If boot metadata is unavailable, generation 3 can consider the **standard** IBM 720 KB or 1.44 MB layout. Image length alone is not enough: both entire FATs must be readable, byte-identical, correctly headed and contain legal allocation values. The root region must be readable, have an intact end marker and contain at least one live nonempty file with a terminated allocation chain consistent with its size. Surviving recognizable BPB fields must not contradict that layout.

This is a corroborated **standard-layout hypothesis**, not exhaustive exclusion of every custom filesystem of the same size. The report records that limitation and the supporting metadata LBAs/root-entry offsets. It does not reconstruct a boot sector, mark unreadable LBA 0 as good, modify the source image or certify customer delivery. Existing chain, ownership, name and unreadable-data checks still apply independently to every file. Empty/deleted-only/root-directory-only or otherwise insufficiently anchored roots cannot establish this fallback. Fully populated roots without an end marker are conservatively refused by this initial fallback, even though they may be valid FAT roots; the normal readable-BPB parser still supports them.

FAT field/chain semantics follow the [Microsoft FAT specification, sections 3–6](https://www.scs.stanford.edu/~zyedidia/docs/_other/fat.pdf); the restrictive fallback rules are FluxVault's recovery policy, not a guarantee in that specification.

## Where a file is broken

The report's `analysis.unrecovered_files` retains skipped file candidates, their allocation/name evidence and the rejection reason. Their `record.sha256` is empty: no complete payload was verified or exported.

- `unreadable_ranges`: logical file byte offsets, exact lengths and acquisition-reported unreadable source LBAs. Adjacent logical holes are grouped even when their physical clusters are fragmented; the final sector is clipped to the recorded file size.
- `unmapped_tail_bytes`: file bytes for which incomplete allocation metadata could not locate a source sector. No LBA is invented.
- Other rejections, such as ownership conflicts or inconsistent sizes, remain explicit even when no unreadable data range is present. An empty hole list does **not** certify the file.

Missing directory regions remain separately recorded; names/entries hidden in them are not invented. Generation 4 can salvage self-contained validated embedded objects from readable partial-file runs, preserving their parent association and raw extents without exporting the incomplete parent. General raw partial-fragment export remains open.

## Saved-evidence checks — 2026-10-06

| Evidence | Result |
| --- | --- |
| Saved pilot 005 / 007 / 009 / 012, boot loss simulated **in memory only** | 3 / 19 / 1 / 11 files retain identical hashes, names, sizes and data LBAs: 34 payloads total |
| Saved pilot 017, same simulation | Fallback conservatively refuses the populated root's absent end marker; its original readable BPB still yields 15 complete files and one skipped file |
| Copied historical 009 from the original customer ZIP | Boot sector unavailable; standard DD layout inferred from surviving structures. Completed DMDE map records 954 bad sectors. The live `Goldberger.jpg` chain has nine logical holes totaling 4,608 bytes, so no complete file is exported; 30 deleted directory slots remain unrecovered |
| Copied current pilot 017 | `ajanlat2.doc` is blocked by a 512-byte hole at logical file offset 17,920; the other 15 files remain available |
| Generation/restart compatibility | Version-2 files/report remain hash-verifiable after version-3 publication; managed reuse retains the inference warning |

The historical signature-extracted files are not a validated recovery-yield baseline: carving/deleted content and files containing unreadable acquisition bytes must be distinguished from intact reachable files. The new report explains why the historical JPEG cannot yet be called complete; it does not claim to have recovered the old carved document.

Copied validation projects: `C:\Users\User\Desktop\FluxVault-Test\Offline-Damaged-FAT12-Checkpoint-20261006` and its `Archived-009` subproject. Original pilot images/captures and the original ZIP remain unchanged. The earlier `Offline-Damaged-FAT12-Validation-20261006` folder is an intermediate validation snapshot, not a delivery package.

To rerun the read-only saved-pilot regression from the repository:

```powershell
$env:FLUXVAULT_FAT12_TEST_PROJECT = 'C:\Users\User\Desktop\FluxVault-Test\Customer-020-Pilot-20261006'
cargo test --lib real_saved_pilot_payloads -- --ignored --nocapture
```

Next: deeper evidence-ranked metadata reconstruction, explicit deleted-file recovery, additional signatures and partial-fragment salvage. Bounded orphan-chain/signature recovery is implemented in generation 4; full historical yield equivalence and 136-disk acceptance remain separate work.
