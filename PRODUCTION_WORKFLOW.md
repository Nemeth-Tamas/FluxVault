# Production: feed disks, finish automatically

`fv production start` joins the existing read-only scanner, automatic recovery and
background file processing to verified ZIP creation. Every required label and
USB-to-GW transfer must be saved first. Physical labels/safe swaps remain your
responsibility; technical recovery and conversion choices are automatic within
the supported formats and bounded policy.

## Beginner: a fresh batch

Choose a new folder; do not reset/reuse a mislabeled or historical batch.

```powershell
fv init 'C:\Users\User\Desktop\FluxVault-Test\My-New-Production-Batch'
cd 'C:\Users\User\Desktop\FluxVault-Test\My-New-Production-Batch'
fv production start --last-disk 136 --sound on
```

Insert the displayed protected floppy, type its number, wait for its saved/swap
cue and repeat. Default: the tested single Greaseweazle station, selector B.
No policy/profile file is needed for standard IBM 720 KB/1.44 MB disks.
Recovery/conversion/reporting and lossless capture packing run automatically.

After the final saved read, remove disks when instructed. Finishing continues
offline and prints the ZIP path/SHA-256. Default output: a **sibling**
`My-New-Production-Batch-Delivery` folder, created automatically. Keep the project;
the ZIP is a snapshot, not a replacement for working evidence.

## Stop and return later

During feeding, `QUIT` drains saved work but never packages an unfinished batch.
`STOP` or Ctrl+C cancels active work. From another terminal:

```powershell
fv stop --project 'C:\Users\User\Desktop\FluxVault-Test\My-New-Production-Batch'
```

Wait for STOPPED **and drive activity to cease** before moving media. Return to
the same project:

```powershell
fv production resume --sound on
```

`production start` also resumes. Range, station options, destination and requested
worker count are saved. Follow the current numbered prompt; never reset the cursor
or edit journals. Interrupted finishing rechecks acquisitions and resumes offline
without another physical read. Closing the console leaves the last known phase,
not proof of completion. Interrupted ZIPs stay partial; restart creates a new
immutable archive, not an in-place repaired ZIP.

```powershell
fv production status
fv run status
fv processing status
```

These inspect recorded state offline. Repeating a completed production start is a
no-op returning its **historical** archive receipt, not a fresh hash check. Expert
`fv finalize --destination PATH` creates a newly checked snapshot of saved images.

## Advanced: optional controls

```powershell
# Single-GW shortcut: check each label, then Enter after the swap.
fv production start --last-disk 136 --no-verify --sound on

# Optional USB + GW; exact uN/gN labels remain mandatory.
fv production start --double --write-blocker-verified --last-disk 136
fv production resume --write-blocker-verified

# Save an alternate destination/requested conversion job count on first start.
fv production start --last-disk 20 --destination 'C:\Archives\Customer-Delivery' --conversion-workers 12
```

The destination parent must exist; its final folder is created if absent. Never
select a floppy/device, the project, its descendants or an ancestor. Range: at
most 4,096 labels, starting at the current number on first invocation. Use a fresh
project for different batch options.

`--no-verify`, sound/color and the USB blocker assertion are **per invocation**,
not saved. The shortcut skips label typing only, not evidence checks, and cannot
be used in dual mode. No assertion substitutes for validated USB protection.
Dual feeding retains [station commands](DUAL_SCAN.md); it means USB + GW, not
simultaneous A/B reads on one board. Offline finishing needs no blocker assertion.

`--gw-drive`, `--profile`, `--profile-map` and `--policy` remain single-GW expert
controls. Production excludes acquisition-only/custom-tail/USB-only modes; use
`scan` for those. Existing `scan`/`start` and journals remain compatible and do
not silently acquire the new auto-package policy.

## Archiving policy and results

Choosing production explicitly opts into **archiving partial results with their
attention reports**. Exit 0 means no reported attention in the completed chain;
exit 3 means waiting for disks/transfers or a finished attention archive. Exit 2
is an input/operation error; exit 130 is cancellation. Read the printed phase.
Neither exit 0 nor a verified ZIP certifies all original customer files recovered.
Fatal evidence/tool/package errors still refuse publication.

An entirely unsupported raw-only endpoint preserves verified captures plus
`Reports/RawOnlyEndpoint-*.json` in an attention ZIP, explicitly reporting zero
sector images/extracted/converted files. No decode is fabricated and future
offline recovery is not declared impossible. Mixed raw/decoded batches use normal
image processing with raw exceptions retained. Ordinary `finalize` still blocks
attention without explicit `--allow-attention`, and still requires saved images.

## Checks and limits

One writer/stop-controller generation spans feeding through archive verification.
Endpoint decisions replay hash-verified acquisitions, not a cursor/saved phase;
gaps and pending transfers prevent packaging. Fixtures cover clean/partial/raw-only,
early quit, capture stop, conversion stop/forced close, offline restart, competing
writers and dual transfer gating. Physical 136-disk yield, correct physical labels
and elapsed-time acceptance remain measured tests, not fixture conclusions.
