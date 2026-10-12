# Four historical candidate investigations

`fv benchmark review N --baseline ZIP --json` compares saved acquisition/live FAT chains with the old ZIP's extracted candidates. No media/tools, new extraction, guessed bytes, preferred-image change or archive writes. Exit **3** retains investigation attention. Raw offset matches do not establish live/deleted ownership; block presence is not allocation proof.

Final release SHA-256 `DEF754EBD6B49CD9AC18D97BF816318BB0F3CAD11AC05832DB1ED30B83ACFB6F`. Reference ZIP SHA-256 `5a002153f77523fa73766d333e58e11a0edb4016080d8c1e5ce39dd707de0cf1`. Saved 009/017/020 use `Customer-020-Pilot-20261006`; 058 uses `Customer-136-Dual-20261010`. Existing controlled 009/058 fresh-repeat results remain recorded in the pilot documentation; no new physical reads were needed.

## 009: signature bytes are not automatically whole recovered files

All seven archived candidates are accounted for: two recovery-tool reports, three two-byte `.cdoc-r` stubs, a **617,472-byte Word candidate** and a **77,275-byte JPEG candidate**. The stubs are not whole files; reports are not customer payloads. The two signatures have unproven original ownership, not an automatic deleted/live classification.

The Word candidate exactly occupies old stored-image bytes starting at **119,808 (LBA 234)** through image EOF. The JPEG starts at **7,168 (LBA 14)**. The old complete-extent DMDE log identifies **954 unreadable LBAs**. These candidate spans intersect **940** and **9** unreadable sectors respectively; those same counts of aligned full candidate blocks are absent from readable current-image observations. Thus an exact old raw span can include acquisition placeholders and does not certify intact Word/JPEG content. Neither entire candidate occurs in the new image or exactly matches its reachable FAT payload.

The DMDE log has complete matching extent/accounting, **not an original image SHA certificate**. Its association is explicitly weaker than a hash-bound archiver map. The current clean DD image/reachable JPEG is not claimed equivalent to the older severely damaged image's carving results.

## 017: an exact incomplete live chain

`ajanlat2.doc` (**39,424 bytes**) is a known current live FAT entry with one missing **512-byte** sector, **LBA 68**, at logical file offset **17,920**. It is not a renamed signature or invented deleted candidate. Existing text salvage retains readable content separately; it does not turn that incomplete original into a clean whole file. The old extent-associated map also reports one missing sector.

## 020: not merely summary properties

`Leltár.doc` has different saved image/file bytes. Allocated CFB stream comparison finds **3,722 differing bytes in `/WordDocument`**, beginning at stream offset **99,840**. The first 32 ranges are displayed with an explicit further-range limit. `/1Table`, `/Data`, both summary-property streams and `/CompObj` are byte-identical. This is not a summary-property-only difference, nor proof of who changed the document or semantic equivalence. Its historical image map is unavailable/contradictory; absent map data is not zero bad sectors.

## 058: several allocation/content-stream differences

| Same-named archived/current file | Differing allocated stream bytes |
| --- | --- |
| `jelmez törzs.doc` | `/1Table`: 3,575; `/WordDocument`: 4,094 |
| `jelttablakompl.doc` | document-summary properties: 512; `/1Table`: 7,161; `/WordDocument`: 9,214 |
| `tablak.doc` | `/1Table`: 1,014; `/WordDocument`: 1,023 |
| `MNMbabukolcs.doc` | Allocated CFB comparison unavailable; exact file difference remains recorded |

Collision-renamed versions remain ownership-unproven; temporary/empty members and tool reports are separated from ordinary payload scope. These are not merely summary metadata changes. Missing original map trust and differing images prevent claiming drift cause, semantic equality or new recovered-file parity. Earlier controlled repeat reproduced the current saved image/files, not the historical ZIP edition.

## Reproduce / retained receipts

```powershell
pwsh -NoProfile -File .\scripts\validate-legacy-review.ps1
```

Final release receipt: `C:\Users\User\Desktop\FluxVault-Test\Legacy-Review-20261012-d2023206\Acceptance.json`, four complete JSON reviews, **407 protected image/reference/handoff artifacts unchanged**. Full CRC receipt: `C:\Users\User\Desktop\FluxVault-Test\Flux-CRC-20261012-53e019ed\Acceptance.json`, **1,508 protected artifacts unchanged**, 136 identities retained: **57 complete / 11 partial independently CRC-byte-qualified**, **11 attention/unconfirmed**, **57 no committed flux (USB-only)**. Known HD/DD validate 2,880/1,440 sectors. No independent CRC-valid byte conflict appeared in this cohort; unconfirmed is not a proof of incorrect bytes. Disk 133's existing cross-reader disputes remain attention despite CRC-qualified GW bytes.

The completed customer ZIP remains unchanged: SHA-256 `e674184adb051b8cdc45303039356691833190d6d9983a64671e4f6b12437858`. This investigation adds no whole-file recovery count, does not close historical aggregate reconciliation or actual same-evidence yield parity, and does not certify delivery.
