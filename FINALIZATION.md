# Finish a batch — no floppy needed

Scanning already processes saved files in the background. **Finalization** refreshes recovery, extraction, conversion, the final audit/workbook, and then builds and verifies an immutable archival ZIP. It uses workstation files only.

## Beginner: finish a clean batch

From your existing project folder:

```powershell
New-Item -ItemType Directory -Path 'C:\Users\User\Desktop\FluxVault-Delivery' -Force
fv finalize --destination 'C:\Users\User\Desktop\FluxVault-Delivery'
```

The destination must exist **outside the project**, on workstation storage, not a floppy/device path. No disk needs to be inserted. This command owns the project throughout tool checks, processing, reporting, packaging and verification; competing changes are refused. Status commands remain available.

By default, attention stops automatic packaging. Read the final report; a damaged disk does not become healthy just because some files were extracted.

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

Interrupted ZIPs remain `.partial.zip`. Resume builds a **new** immutable package, not an append/resume-in-place of the old ZIP. If a crash occurred after ZIP completion but before its receipt was saved, an earlier complete archive may remain too; neither is silently overwritten. A completed receipt makes `finalize resume` refuse another run: use a new finalization if you want a freshly checked snapshot.

`finalize status` is read-only, needs no tools, and reads no media. It shows saved phase/settings and whether a registered finalizer currently owns the project. A receipt is historical: status does not rehash the ZIP or certify that files have not changed since completion.

## Advanced details

```powershell
fv finalize --destination 'D:\Archives' --conversion-workers 12 --allow-attention --json
fv finalize status --json
```

Workers accept 1–16, default 4; resource admission can limit concurrency. Tool/output integrity guards and normal recovery provenance still apply. Registered `fv stop` spans the **whole finalization**, with no processing/package ownership gap. Exit 130 means cancelled; fatal failures normally return 2.

The bounded, project-bound `.fluxvault-finalize.json` pointer records phase/options/results. Immutable phase receipts stay under `.fluxvault-finalizations`; malformed, oversized, linked or foreign-project control files are refused rather than silently replaced. Receipts are workstation control state and are not customer package contents. Failed/interrupted runs keep earlier evidence and explain their last known phase. Power loss can leave an in-progress receipt despite no active owner; resume revalidates instead of assuming work is still running.

This command starts when you request it. Automatically finishing/packaging at the scan endpoint and unattended alternate-profile flux searches remain separate TODOs. `processing resume` and `storage resume` remain the controls for their durable background queues. Keep the source project after packaging.

See [final report contents](FINAL_REPORTS.md), [stop/resume](STOP_RESUME.md), and [the CLI reference](CLI.md).
