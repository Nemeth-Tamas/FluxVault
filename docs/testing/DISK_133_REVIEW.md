# Disk 133: saved USB/GW disagreement review — 2026-10-12

The operator confirmed the same physical label. This review uses only saved evidence in `C:\Users\User\Desktop\FluxVault-Test\Customer-136-Dual-20261010`; it does not remove the recorded attention, select an original-byte winner or repair an editable DOC.

## Source binding

| Edition | Image SHA-256 |
| --- | --- |
| USB attempt 001 | `9c1fb5752bc021c5051339e3f2338f0425758db4e1bcbbc8fce89577c7fc232c` |
| GW attempt 002, selected | `dfd19a087364195b9c291a95ba196e86e7a5fa5aa75939a879eb5508b2d13b66` |

The confirmation binds 2,584 agreeing readable sectors and two conflicts. The USB image has 294 unavailable sectors, including boot/FAT metadata, so its filesystem is not independently mapped by this review. The selected GW image has no missing sectors but retains the cross-reader conflict evidence. Zero missing is not equivalent to undisputed content.

## Exact disagreement scope

| LBA | Observed differences | Mapping and consequence |
| --- | --- | --- |
| 17 | 170 differing bytes | Second FAT copy, sector 7. 113 declared FAT12 cluster entries differ; none has a known allocation claim in the selected analysis. GW's corresponding primary/secondary FAT sector bytes agree; USB's do not. These are observations, not proof of the historical allocation or permission to discard either edition. No currently mapped live-file metadata dependency uses this sector. |
| 773 | 426 differing bytes | `V_xB5MOSANG.DOC`, logical file bytes 11,776–12,287; `/WordDocument` stream bytes 1,024–1,535. Both observed versions remain retained. |

The file is a compound Word6 document (`wIdent=0xa5dc`, `nFib=0x65`), 19,968 bytes. Strict source-bound CFB mapping identifies its streams. A narrow legacy FIB/complex-piece probe covers **1,519 declared main positions in 229 pieces**, without assuming a character encoding. The disputed sector is **outside every mapped main-body byte extent** and outside the required main-mapping metadata.

All 1,519 ordered main-byte observations agree with readable USB bytes at the selected GW map's physical positions; zero differ or are unavailable. Repeated fast-save references are ordered position observations, not a unique-byte/file-yield count. This does not independently establish the USB file allocation/name because its boot/FAT evidence is incomplete. Main-body byte agreement also does not establish original formatting, objects, fields, other stories, rendering or semantics. The remaining differing Word-stream material is not classified as harmless, corrected or an original-content winner.

## Reproduce without media or project writes

```powershell
fv recovery investigate 133
fv recovery investigate 133 --attempt 2 --json
fv recovery trace 133 'V_xB5MOSANG.DOC' --attempt 2 --json
```

JSON includes both source/sector hashes, all differing bytes, FAT-copy values/claims, stream offsets, required metadata LBAs, ordered main-piece extents/hashes and the qualified cross-reader comparison. Confirmation/source/map changes refuse. [Command scope and limits](../recovery/RECOVERY_INVESTIGATION.md).

The broader investigations into independent flux CRC, editable-document repair, historical script/DMDE parity and unmapped/deleted ownership remain separate. This closes the specific saved disagreement **review**, not those recovery/certification goals.
