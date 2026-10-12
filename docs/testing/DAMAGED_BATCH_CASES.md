# Saved damaged-file case review — 2026-10-12

Collection: `C:\Users\User\Desktop\FluxVault-Test\Customer-136-Dual-20261010`.

`recovery investigate all` reproduced **136 inspected labels**, **13 known live-file dependencies** on missing/disputed sectors and **16 incomplete/ambiguous allocation chains**. Their union is **17 distinct file records across 13 disks**, not 29 lost files. The full underlying impact report retains one unknown filesystem and directory/tail/skipped-entry context elsewhere in the collection.

## Every union record

Main-text counts are character positions within supported Word mappings, not whole-file recovery or original semantics. RTF is a readable prefix only. Source filenames/content remain unchanged.

| Disk | Record | Observed supported outcome |
| --- | --- | --- |
| 017 | `ajanlat2.doc` | 3,229/3,229 main positions; no mapped main-text gap. A 512-byte hole at file offset 17,920 still makes the original container incomplete. |
| 022 | `nka261056994.rtf` | 5,318 readable UTF-16 positions; 18 uninterpreted symbol glyphs / 19 separate text segments. Stops at byte 143,867 before the acquisition hole at 143,872; no parser state guessed across it. |
| 024 | `BIOFIZ.DOLG.doc` | 1,053/1,053 main positions. 101,376 file-tail bytes remain allocation-unmapped; full main text is not a complete DOC. |
| 024 | `~WRD1848.tmp` | Recorded zero length; the current traversal rejects its cluster-0 chain. No declared payload bytes are missing. This is not a lost-document claim or a new healthy-file promotion. |
| 027 | `LBA2.doc` | 25,672/25,928 main positions; 256-position gap. Unrelated compound-directory holes retain their warning. |
| 033 | `ORSZAGOS_ADATLAP_2006.doc` | 22,532/23,044 main positions; 512-position gap, exact source extents retained. |
| 033 | `osszefoglalo_05.doc` | 2,878/2,878 main positions; original container still has a 512-byte hole at file offset 19,456. |
| 040 | `Vincent Van Gogh - Szövőszék.jpg` | Raw readable fragments retained; holes at file offsets 4,608 and 23,040. No invented JPEG blocks/pixels or complete-image claim. |
| 052 | `fejléc_logoval.doc` | 1,792/1,792 main positions through the warned common-prefix mapping. Optional extension-count/effective version and full container remain unknown. |
| 066 | `Sajtó tájékoztató/Sajtó_ismertetők/10.23. 22h BOF-party.doc` | 2,857/3,113 main positions; 256-position gap. |
| 066 | `Sajtó tájékoztató/Sajtó_ismertetők/10.24. 19h30 Sir Peter Maxwell Davies.doc` | 3,482/3,482 main positions; original container still has a 512-byte hole at file offset 13,824. |
| 066 | `Sajtó tájékoztató/Sajtó_ismertetők/10.24. 20h kis szeptim kamaraegyüttes.doc` | 1,455/1,711 main positions; 256-position gap. |
| 077 | `2C454000` | File-size/terminated-chain disagreement; 1,536 mapped bytes, no unmapped declared tail. Allocation/format remains unresolved, no complete-original promotion. |
| 078 | `System Volume Information/IndexerVolumeGuid` | Windows metadata, excluded from normal customer payload scope; its invalid allocation remains visible, not a lost customer-document claim. |
| 123 | `TELEFON1.doc` | 18,747/18,747 main positions; original container still has a 512-byte hole at file offset 86,016. |
| 128 | `vasalók.rtf` | Binary Word content despite extension: 683/939 main positions, 256-position gap. Not falsely decoded as RTF. |
| 133 | `V_xB5MOSANG.DOC` | Word6 byte map: 1,519 ordered main-position bytes agree across readable observations; disputed sector outside mapped main ranges. No assumed encoding, repaired DOC or original-byte winner. [Exact review](DISK_133_REVIEW.md). |

## What the shared recovery pipeline preserved

The 11 supported binary Word records yield **84,380 readable main positions** out of 85,916, with **1,536 missing positions**. Six retain every declared main position, five have explicit gaps. The RTF prefix's 5,318 positions are separate; the Word6 raw-byte map is not added to decoded-text totals. These are source-bound text results, not new intact originals, normal Office jobs or a deduplicated recovery-yield measure.

Actual release `recovery documents N` on all 13 reviewed disk labels published/reused current Hungarian Word-v5/RTF-v3 editions through the same native services. Existing v2/v3/v4 evidence, source images, extraction payloads/inventories and the completed root delivery snapshot were unchanged. **2,474 earlier evidence/source/handoff files** matched their before/after SHA-256. The release investigation took **10.23 seconds** on this machine; this is diagnostic timing, not scanning or production throughput.

Raw machine receipts are retained locally under `C:\Users\User\Desktop\FluxVault-Test\Batch-Case-Review-20261012-7c131b3b` (`BatchInvestigation.json`, per-disk `Recovery-NNN.json`). No capture/customer payload or ZIP is added to Git. New editions are separate Recovery generations; the earlier completed ZIP remains its historical snapshot until a new finishing run is explicitly chosen.

Final release repeat: **10.12 seconds** for investigation; all **2,581** files present before the repeat, including the newly published editions, remain unchanged. Current receipts: `C:\Users\User\Desktop\FluxVault-Test\Batch-Case-Review-20261012-74098195`. Current executable SHA-256: `F480B8F64B4AD2A6C33960E8D3D24A8AF96C8F2CB7D55D215619369EB07B3648`.

## Reproduce

```powershell
fv recovery investigate all
fv recovery investigate 133 --json
fv recovery documents 33
```

Developer saved-image acceptance (optimized replay, no media):

```powershell
$env:FLUXVAULT_INVESTIGATION_PROJECT = 'C:\Users\User\Desktop\FluxVault-Test\Customer-136-Dual-20261010'
cargo test --locked --release --lib saved_136_case_review_replays_all_gaps_and_133_disputed_versions -- --ignored --nocapture
```

[Command/safety bounds](../recovery/RECOVERY_INVESTIGATION.md). The two specific saved-case investigation checklist items can close with these explicit outcomes. Broader editable-document repair, additional formats, severely damaged recovery, independent donor CRC and historical yield parity stay separate; missing bytes were not guessed to improve a percentage.
