# Bring an old script archive into FluxVault

Already have a ZIP made by the original scripts? You can keep using its images and recovered files **without scanning the floppies again**. This is an optional migration command, not an extra step for new scans.

## Beginner: preview, import, inspect

Use the current FluxVault executable or refresh your installed `fv` after building. Choose a **new destination folder** whose parent already exists. Do not run `fv init` first: import creates the project itself.

```powershell
$archive = 'C:\Users\User\Desktop\randomprojectsillneverfinish\FluxVault\TextilMuzeum_Floppy_Archive_20260920_115210.zip'
$restored = 'C:\Users\User\Desktop\FluxVault-Test\Script-Archive-Imported'

# Optional preview: checks structure/index, creates nothing.
fv project import --source $archive --destination $restored --plan

# Copy and verify the archive into the fresh project.
fv project import --source $archive --destination $restored

Set-Location $restored
fv status
fv disk list
fv disk show 7 --details
Get-Content .\Reports\LegacyImport.txt | Out-Host -Paging
```

**Exit 3 is expected after a successful import.** It means historical recovery evidence needs attention, not that copying failed. Exit 0 is a successful preview; exit 2 is an input/copy/publication error; cooperative cancellation exits 130.

The printed summary is plain text. It lists every image's historical status, actual saved size, reported bad-sector count and counts of files in its old extracted/converted folders. JSON contains the detailed claims; CSV lists every copied file's size and SHA-256. No Excel or Office application is needed.

## What is preserved

- Original bytes under `Images`, `Logs`, `Extracted`, `Converted`, `Recovery` and `Reports`, plus root `README.txt` if present. Supported numbered image extensions are `.bin`, `.img` and `.ima`. Partial image files are retained but never counted as completed acquisitions.
- Original `ArchiveIndex.csv`, old workbooks, DMDE file lists, recovered files, conversions and logs. The importer never executes archive contents or invokes acquisition, extraction or conversion tools.
- Original image sizes, including DD and truncated/severely damaged images. Nothing is padded to 1.44 MB and no missing bytes or geometry are invented.
- A new `project.json` with the chosen folder name and a cursor one above the highest complete image label. This cursor is a convenience, **not evidence that a new physical disk was read**. Filesystem timestamps are those of the import, not reconstructed historical acquisition times; original recorded dates remain in logs/index rows.

The source ZIP is opened read-only. New reports are `Reports/LegacyImport.txt`, `Reports/LegacyImport.json` and `Reports/LegacyImportFiles.csv`. The latter includes original members, not the newly generated metadata/reports.

## Historical claims are not new certificates

Each copied member is fully read through ZIP CRC verification, synced and independently rehashed. All copied members are checked again before publication. Source ZIP hashes before/after planning and after copying must agree.

This proves the copied bytes match the archive. It **does not prove** an old file was fully recovered, an old PDF matches its source, an old log describes these exact image bytes, or the numbered ZIP image matches a physical floppy label. Imported files remain **legacy/manual recovery**, not freshly managed native recovery or verified Office conversions. No acquisition sector map or managed extraction marker is fabricated.

Recorded image hashes in logs/index rows are compared separately with actual copied bytes. Mismatches remain visible, never silently corrected. Log statuses and archive-index status totals are reported separately; missing logs remain unknown. Folder counts include auxiliary files and are not recovered-file yield. All imports retain legacy attention, even when old imaging logs say OK.

Existing saved-image inspection/recovery commands remain available. Import itself performs **no automatic processing or delivery certification**. For a new physical benchmark use a separate fresh project; do not relabel an old scan or reset this archive to 001.

## Advanced: guards and interruptions

`--json` returns the command result on stdout; progress stays on stderr. Only `--source`, `--destination`, `--plan` and `--json` apply to this command. Duplicate options, physical flags and `--project` are refused. Destination must be absent, outside existing FluxVault projects and on workstation storage, not a floppy/device path. It must have an existing regular parent.

Preview checks archive names, declared sizes and the optional CSV index; **it does not validate every member's payload or CRC**. Applying performs those checks while copying. Unsafe Windows paths, traversal, alternate streams, reserved device names, symlinks, case/prefix collisions, native control/managed marker files and reserved import reports are refused. Bounds: 100,000 entries, 4 GiB source/expanded total, 64 MiB per member, 8 MiB per legacy log, 4 MiB per complete floppy image and 1–4,096 complete image labels. Legacy DMDE map parsing has independent bounded expansion/stop checks.

Apply takes an exclusive destination lock and copies into a sibling `.fluxvault-import-...partial` staging container. The destination project appears only after successful verification and a no-merge directory rename. Another import cannot own the same destination concurrently. Existing destinations are never overwritten, including on repeat runs.

Ctrl+C stops at a safe copying/parsing boundary. During copying, the printed staging project can also be targeted by `fv stop --project 'FULL-STAGING-PROJECT-PATH'`; wait for the command to finish. On interruption/failure, incomplete copied evidence and `IMPORT-INCOMPLETE.json` remain in staging. No final destination is published. **Import does not yet resume or merge partial staging**: rerun against the still-absent destination to start a new independent stage, retaining the old one for inspection.

Successful imports retain a small sibling destination lock and staging container holding the staging run-control history. They are not live owners once the command exits; the published project's `fv run status` is inactive. Nothing is recursively removed by import.

## Measured original-archive check — 2026-10-10

The repository's original `TextilMuzeum_Floppy_Archive_20260920_115210.zip` passed an isolated temporary-project CLI import: **136 images, 6,261 original files, 683,329,461 expanded bytes**. Every copied member was independently compared with its original ZIP member, and the source archive hash stayed unchanged. Disk 009 stayed **737,280 bytes**; disk 133 stayed **417,792 bytes**, with its log extent disagreement flagged.

Current primary-log parsing yields **102 OK / 28 PARTIAL / 6 MISSING LOG**. The archived index separately contains **126 rows: 88 OK / 38 PARTIAL**. There are **38 historical hash/extent discrepancies: three log hashes, 34 index hashes and one log extent**. These differ from the previously documented 94/42 aggregate; historical claims need reconciliation, not a declaration of recovery improvement. The discrepancies are old claims versus actual archived image bytes, **not failed copy hashes or newly lost files**. No fresh extraction, customer-media test or customer delivery certificate follows from this copy test.
