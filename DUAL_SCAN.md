# Two drives, one numbered batch — coordinator groundwork

## Available commands

Plain `fv scan` stays **Greaseweazle-only**. Dual mode is opt-in, not the default.

```powershell
fv scan --usb --count 20 --write-blocker-verified
```

`--usb` selects the existing USB-only loop, default Windows A: (`--drive LETTER:` overrides). Type the displayed number (`001` or `1`), or `QUIT`; legacy `READ` still works. All existing USB protection/probe/read-only gates remain. This does not add the GW background pipeline or automatic format discovery to USB. `--count` is a session count; USB does not yet support `--last-disk`.

To preview the future dual setup, with no floppy inserted:

```powershell
fv scan --double --plan --last-disk 136
fv production status --json
```

The preview opens no drives, probes no tools, creates no coordinator and saves no scan settings. `--drive A:` / `--gw-drive B` can annotate it. **Live `scan --double` is not ready and deliberately refuses.** Dual mode rejects `--no-verify`: switching to an earlier recovery disk needs exact label confirmation.

## Agreed speed-first behavior

Both stations take **fresh disks**; GW does not sit idle waiting for USB failures. One coordinator offers the next unreserved label to either ready station, without fixed odd/even assignments. GW uses its fast-first automatic recovery stages when errors appear. USB preserves partial images/maps, says **SET ASIDE NNN FOR GW**, then continues fresh feeding without extended flux recovery.

At GW, entering an earlier queued USB label selects recovery for that disk while USB keeps working. Example: USB takes 001, GW takes 002; USB saves 001 partial, the operator removes it and feeds USB 003. Once GW releases 002, entering `001` selects its recovery. Wrong, clean-historical or in-use labels must not silently read/renumber a disk.

The core can switch an **unread fresh GW offer** to that queued label atomically. The unused fresh label becomes available again; its old ticket is invalidated. This shortcut cannot discard a started/interrupted read or an evidence-bearing reservation.

Preferred terminal design for the next slice: **two station views sharing one coordinator**, so prompts/progress do not mix. They must not be two ordinary independent scans competing over the project cursor. A main status view can aggregate both. No additional console is launched by this checkpoint.

## Implemented core

The backend-independent Rust coordinator reserves labels durably and tracks independent generation-tagged station custody. Claiming a label does not authorize a read: confirmation requires its exact number and physical protection assertion. USB partials become GW-selectable only after confirmed removal. Failed/in-flight operations keep their label; reopen marks unfinished reservations/reads interrupted and requires reconfirmation. Saved results retain removal obligations; completed/queued results survive reopen.

Completed receipts bind the original acquisition backend, image/metadata/log hashes, sector count and complete bad-sector map. Transfers require matching geometry, at least one mutually readable sector and agreement of **all** mutually readable sector bytes. This is conservative consistency checking, not proof of physical identity. Disagreement/unknown shared bytes preserve evidence and refuse queue completion.

One workstation owner serializes atomic bounded journal transitions in `.fluxvault-production.json`; actual station work runs outside the short mutation lock. Malformed/inconsistent/externally edited controls are refused. Images acquired by another command while stopped are excluded from fresh-label allocation. Limits: 4,096 records/occupied labels and 8 MiB control data.

The core does not launch readers/processing, reset the ordinary project cursor, automatically import old archive partials, or replace the existing GW scan journal. `production status` checks saved receipts offline. It distinguishes an absent owner, but an ownership probe cannot prove a physical reader is active.

## Next wiring boundary

1. Connect existing USB/GW backends with physical-device reservations and staged identity checks **before exposing new captures to background extraction**. A USB failure that cannot produce an image remains interrupted, not a pretend partial success.
2. Share one project writer with processing/packing. Do not take a second owner or hold the coordinator mutation lock during a physical read/Office conversion.
3. Connect station-specific label/quit/reseat prompts and removal/transfer assertions; ordinary swap confirmation should cover removal without an extra routine command.
4. Reconcile backend completion after interruption without unnecessary physical rereads; validate process-kill/cancellation and source-preserving restart.
5. Test protected small live cohorts before enabling live `scan --double` or claiming a speed gain.

## Validation

Synthetic saved-image tests cover both fresh stations, earlier-label USB recovery, removal gates, exact confirmations, stale tickets, restart states, wrong backend/changed evidence, cross-station conflicts, ownership, control edits and endpoint consistency. A barrier-controlled mock-worker test proves USB can finish another disk while GW is still busy. A 136-label model reopens midway, recovers 14 queued USB partials and reopens with all identities intact.

These are state-machine tests on tiny disposable acquisitions—not real simultaneous reads, a physical throughput benchmark or six-hour acceptance. No customer media is accessed for this checkpoint.
