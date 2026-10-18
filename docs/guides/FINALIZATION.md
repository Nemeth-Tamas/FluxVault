# Finish a batch — no floppy needed

## Recommended: one command

From the project folder:

```powershell
fv finish
fv package verify
```

`finish` refreshes the saved-image pipeline and creates a new verified archival edition. It automatically creates sibling `<project-folder>-Delivery`, retains attention/partial results with their warnings, and publishes the ready ZIP, checksum, `FINAL_*` reports and `START_HERE-<edition>.html` beside `project.json`. Nothing reads a floppy. Earlier ready editions stay preserved. Exit 3 means a verified archive can still contain attention, not that its ZIP is broken.

Extract the ZIP and open **START_HERE.html** for a Hungarian-by-default disk-by-disk customer page. Direct PDF/editable links require a current VALID report entry and a matching archived SHA-256. Original/evidence folders and limitations remain separate; the page is not a completeness certificate. Use `fv finish --language en` for English or `--conversion-workers 12` for a requested worker count (resource admission still applies).

Use `fv finish status` to inspect saved state, `fv stop` from another console to cancel active finishing, and `fv finish resume` to continue afterward. If already completed, resume verifies/publishes the same recorded ZIP without rerunning conversions or rebuilding it. A new `fv finish` deliberately refreshes the snapshot. `fv package verify` checks the ready snapshot without changing it.

The older expert `finalize` command below remains available with its original strict attention-blocking default and explicit destination. `finish` uses the production-style permission to archive partial evidence; it does not bypass integrity failures or certify damaged content.

Scanning already processes saved files in the background. **Finalization** refreshes recovery, extraction, conversion, the final audit/workbook, and then builds and verifies an immutable archival ZIP. It uses workstation files only.

Starting a fresh batch with `fv production start --last-disk N` now calls this finishing chain automatically at its checked endpoint, with explicit partial-results archival policy. `production resume` continues its saved options and offline tail; default output is sibling `<project-folder>-Delivery`. Ordinary `finalize` below keeps strict default attention blocking. Entirely raw-only production endpoints use a separate verified-capture/zero-file-yield attention archive, not a bypass for ordinary image finalization. [Production guide](PRODUCTION_WORKFLOW.md).

## Beginner: finish a clean batch

From your existing project folder:

```powershell
New-Item -ItemType Directory -Path 'C:\Users\User\Desktop\FluxVault-Delivery' -Force
fv finalize --destination 'C:\Users\User\Desktop\FluxVault-Delivery'
```

The destination must exist **outside the project**, on workstation storage, not a floppy/device path. No disk needs to be inserted. This command owns the project throughout tool checks, processing, reporting, packaging and verification; competing changes are refused. Status commands remain available.

By default, attention stops automatic packaging. Read the final report; a damaged disk does not become healthy just because some files were extracted.

Successful finishing places the verified ready ZIP/checksum and `FINAL_*` final report copies **beside `project.json`**, automatically. The outside-project original remains preserved. `DELIVERY_READY.json` binds all copies to that exact ZIP and archived report generation; copies are excluded from future package inventories, so packages cannot include themselves. Each report's suffix identifies its archive edition, preserving previous editions without replacing unrelated/user-edited files. Allow extra storage for one ZIP-sized copy.

For a previously finished batch, or handoff copying interrupted after the archive receipt was saved:

```powershell
fv package publish
```

This verifies/copies the recorded ZIP and its **embedded** final reports without media, tools, extraction, conversion or archive rebuilding. It does not refresh an old archive to match newer project changes: run a new finalization for that. Linked paths, changed archive/copies, foreign pointers and unowned filename collisions refuse; partial publication stays resumable and only all-verified copies receive a ready commit receipt. Workstation filesystems must support safe non-replacing hard-link promotion for staged copies (tested on Windows NTFS); this never links a handoff copy to its original archive/evidence.

## Check an existing handoff without changing it

```powershell
fv package verify
fv package verify --project 'C:\full\project\path' --json
```

This read-only check rehashes the ready ZIP beside `project.json`, replays its embedded final-report inventory, and checks the front-and-center reports, checksum and matching ownership receipt. It does not create locks/files, repair a missing copy, invoke tools, inspect a floppy or require the outside-project original ZIP. Existing writer locks are respected; wait for processing/publication to finish before checking.

Exit **0** confirms the published snapshot's integrity only. It does not clear attention, certify damaged documents, compare with newer project files or refresh the archived edition. A missing/changed/foreign copy returns an error without replacing it. Use `package publish` for interrupted handoff copying; use a new finalization for newer project contents.

## Archive partial results explicitly

If you want a complete archival snapshot of what was recovered, including limitations:

```powershell
fv finalize --destination 'C:\Users\User\Desktop\FluxVault-Delivery' --allow-attention
```

Or, after a clean-only finish was blocked:

```powershell
fv finalize resume --allow-attention
```

This is permission to **archive partial results**, not to ignore an integrity error, invent missing bytes, repair documents by guessing, or certify customer delivery. Current delivery artifacts can include outputs flagged by the validation report; keep their warnings with them. Fatal processing/evidence/path/package failures still stop. The archive includes the hash-bound final report bundle and file inventories.

An archive with unresolved attention returns **exit 3**, even after successful ZIP verification. In JSON, its status is `verified_archival_zip_with_attention`, and `customer_delivery_certified` remains `false`. Clean archives return 0 with `verified_archival_zip`; these also do not certify every historical byte or document's meaning. Blocked clean-only finishing returns 3 with `blocked_by_attention` and no archive.

## Stop and continue later

From a second console:

```powershell
fv stop --project 'C:\full\project\path'
```

Or use Ctrl+C in the finishing console. Wait for **STOPPED**. Unlike scanning, finishing does not read typed commands from stdin: typing STOP into that console is not a finishing control.

When ready, from the same project:

```powershell
fv finalize status
fv finalize resume
```

Resume uses the saved destination, worker count and partial-archive policy. It **rechecks** the current inputs and outputs through the shared pipeline; valid completed extraction/conversion products can be reused. It does not trust old summaries, skip validation, reread a floppy, or reset scan numbering. To change workers/destination or return to clean-only policy, run a new `finalize --destination ...` command instead.

Interrupted ZIPs remain `.partial.zip`. Resume builds a **new** immutable package, not an append/resume-in-place of the old ZIP. If a verified archive receipt was saved before handoff interruption, resume verifies/publishes its existing root copies instead, preserving the saved processing attention and skipping tools/rebuilding. If a crash occurred after ZIP completion but before its receipt was saved, an earlier complete archive may remain too; neither is silently overwritten. A completed receipt makes `finalize resume` refuse another run: use `package publish` to recheck/copy its recorded snapshot, or a new finalization for a freshly processed snapshot.

`finalize status` is read-only, needs no tools, and reads no media. It shows saved phase/settings and whether a registered finalizer currently owns the project. A receipt is historical: status does not rehash the ZIP or certify that files have not changed since completion.

## Advanced details

```powershell
fv finalize --destination 'D:\Archives' --conversion-workers 12 --allow-attention --json
fv finalize status --json
```

Workers accept 1–16, default 4; resource admission can limit concurrency. Tool/output integrity guards and normal recovery provenance still apply. Registered `fv stop` spans the **whole finalization**, with no processing/package ownership gap. Exit 130 means cancelled; fatal failures normally return 2.

The bounded, project-bound `.fluxvault-finalize.json` pointer records phase/options/results. Immutable phase receipts stay under `.fluxvault-finalizations`; malformed, oversized, linked or foreign-project control files are refused rather than silently replaced. Receipts are workstation control state and are not customer package contents. Failed/interrupted runs keep earlier evidence and explain their last known phase. Power loss can leave an in-progress receipt despite no active owner; resume revalidates instead of assuming work is still running.

This expert command starts when requested; production already invokes finishing/packaging automatically at its checked endpoint. Broader unattended alternate-profile searches remain separate TODOs. `processing resume` and `storage resume` remain controls for their durable background queues. Keep the source project after packaging.

See [final report contents](FINAL_REPORTS.md), [stop/resume](STOP_RESUME.md), and [the CLI reference](CLI.md).
