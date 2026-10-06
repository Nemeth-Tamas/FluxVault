# Compare recovery with the original archive

This is an offline, source-file comparison. It never opens a floppy, queries the board, extracts the reference ZIP onto disk, converts documents or changes saved payloads. Run it after a scan/background drain, not while the project is owned by another worker.

```powershell
fv benchmark compare --baseline 'C:\Users\User\Desktop\randomprojectsillneverfinish\FluxVault\TextilMuzeum_Floppy_Archive_20260920_115210.zip'
```

The command writes uniquely named `Reports/BaselineComparison-*.json` and `.csv` snapshots. Add `--json` for machine-readable stdout; progress goes to stderr. Exit 0 means all in-scope reference payloads matched; 3 means changed/missing content or an empty comparison scope needs attention; 2 means the comparison could not be trusted/completed. This is **not** customer-delivery certification.

## What is compared?

- Only acquired disk numbers in this project. Unscanned reference disks are listed separately, never counted as missing.
- Original recovered files under reference `Extracted/NNN`, not duplicate Office derivatives under `Converted`.
- SHA-256 and size, with one-to-one matching within each disk. First match normalized paths and bytes; then renamed identical bytes; finally identify same-path changed bytes. One current file cannot satisfy multiple reference copies.
- Preserved older managed extraction generations are still usable after their image/inventory hashes verify. Operator recovery is labeled unbound, not silently certified.
- Per-disk source-image hashes are compared separately. Different image bytes prevent a claim of identical-evidence recovery yield; file matches do not establish why an image changed.
- Empty/temporary files and tool/OS metadata do not inflate normal payload totals. Diagnostic rows retain empty/temporary files; recovery tool reports are counted separately.

## Deleted recovery is off by default

The customer workflow should recover current files automatically, not resurrect every deleted draft/photo. Future native deleted recovery must be explicit opt-in and separately labeled; it is still an open implementation task.

For this comparison, a matching path **and byte count** in DMDE's archived UTF-16/UTF-8 `filelist.txt` can identify a confirmed deleted file. Those payloads are excluded from the default score but retained as `deleted_out_of_scope` rows. The type legend is documented in [DMDE's file panel manual](https://dmde.com/manual/filepanel.html).

```powershell
fv benchmark compare --baseline 'C:\path\reference.zip' --include-deleted
```

**This flag expands comparison scope only; it does not enable deleted-file extraction.** Unknown, signature-carved and ambiguous entries stay visible and in scope. Duplicate DMDE paths can refer to live and deleted versions; collision-renamed ZIP entries cannot safely be classified by guessing the filename. They can still appear as missing/changed even if you do not want deleted recovery. This conservative behavior is explicit in the report, not evidence of a lost live file.

## Saved customer baseline, 2026-10-06

| Saved project | In-scope reference payloads | Identical | Changed | Missing |
| --- | ---: | ---: | ---: | ---: |
| `Customer-020-Pilot-20261006` | 188 | 181 | 1 | 6 |
| `Customer-053-064` | 88 | 81 | 4 | 3 |

These are measured file-byte matches, **not product-completion percentages**. The first cohort's missing entries are five signature candidates on 009 (including three two-byte fragments) and `ajanlat2.doc` on 017; 020 has a changed `Leltár.doc`. Different source-image hashes are recorded for these disks.

The second cohort's changed/missing rows are on 058. Its archive includes live/deleted same-name versions and collision-renamed drafts, so not all seven rows establish live-file recovery failures. Source images also differ. The ten confirmed deleted payloads from 059 are out of scope by default: both live documents match exactly. All eight reference payloads from partial 062 match exactly despite its 14 remaining missing sectors.

The reference ZIP and saved images/payloads remain intact. Snapshots also retain current-only content, classification/origin, uncertainties and binding verification. Internal comparison/test summaries are not included in customer packages.

## Bounds and refusal rules

Regular workstation files only; no A:/B: sources. Reject unsafe/duplicate ZIP member paths and symlinks, changed archive/image bindings, invalid managed inventories and escaped project directories. Stream member hashes with 16 MiB/file, 2 GiB payload and 100,000-entry bounds; reference archive hashing is capped at 4 GiB. Malformed DMDE classification is refused rather than inventing deleted/live state. Report CSV cells are escaped for spreadsheet safety; JSON retains exact paths.
