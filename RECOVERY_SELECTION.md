# Better recovery, without losing earlier work

FluxVault now chooses a recovery generation by its verified evidence, not just the newest folder name. This is automatic during normal saved-file processing; there is no additional scan policy or operator decision.

## What becomes preferred?

For the acquisition already selected by the project's sector-map ranking, FluxVault checks every managed recovery generation against its source image, inventory and native provenance.

A later generation can replace the current preference only when it retains:

- Every earlier readable payload, counted by SHA-256, byte length and duplicate multiplicity.
- Every earlier reachable-file payload; moving the same bytes into signature-only recovery is not an upgrade.
- At least as much validated long-name evidence.

Extra coverage or stronger naming evidence promotes the candidate. Equivalent evidence uses the newer-generation tie-break. Regressions and incomparable candidates remain intact as alternatives; they are not silently merged. Changed source/output bytes refuse selection rather than quietly falling back.

`Reports/RecoverySelection-NNN-HASH.json` records candidates, counts, inventory hashes and reasons. Identical decisions reuse identical snapshots; different decisions create new reports. Manifest, conversion and extraction-presence checks use the shared selector. Existing operator recovery inputs remain preserved and retain the established manual-input behavior.

This compares generations of **one saved acquisition**. It does not prove physical disk identity, rank different scans by recovered-file count, certify an inferred filesystem, or turn partial fragments into complete documents.

## Tidier delivery copies

When a filename improves, an earlier program-created original need not remain duplicated in `Converted` forever. Conversion planning now records ownership and can move that obsolete copy into:

```text
Recovery/DeliveryQuarantine/<cleanup-id>/<disk>/<old-path>
```

Retirement requires an unchanged, previously owned original and verified equivalent replacement bytes on the same disk, retaining duplicate counts. Disappearing disks or reduced payload coverage cannot authorize removal. New original copies use non-overwriting creation; quarantine moves also refuse existing destinations.

Edited files, untracked/pre-ledger copies, conflicting quarantine destinations, needed paths and old Office derivatives are preserved. FluxVault does not retroactively claim ownership of files it finds already present. Prior `Extracted` generations and source images never move.

Ownership history and pending intents survive interruption. Restart can finish a pending move or recognize its already-moved, hash-matching copy. New plans protect newly needed paths from stale intents. `Reports/DeliveryCleanup-HASH.json` explains each outcome; edited control/audit history is refused, not overwritten.

The quarantine is local and recoverable, not a trash purge. Its exact destination appears in the cleanup report; a preserved file can be copied elsewhere if needed. It is excluded from customer ZIPs, while selection/cleanup reports are included. Earlier forensic extraction is still archived. Prior Office derivatives are not automatically retired by this original-copy feature.

## Ordinary commands

From the project folder:

```powershell
fv process
```

For offline original-copy planning without running LibreOffice:

```powershell
fv conversion plan
fv conversion plan --json
```

The plan reports `retired_mirrors`, `preserved_obsolete_mirrors` and `cleanup_reports`. Rerunning unchanged work reuses its files and decisions.

## Saved-capture validation

An isolated copy of saved customer 021–032 is retained at:

```text
C:\Users\User\Desktop\FluxVault-Test\Recovery-Selection-021-032-20261007-v1
```

The migration fixture builds a modeled pre-carving generation from actual readable-chain payloads, then restores the genuine generation-4 results. Preferred originals rise **53 -> 64**, adding the existing 11 signature candidates; all 53 earlier payloads remain. Repeated planning reuses all 64. Saved source-image hashes stay unchanged. This validates preference/migration, not newly increased recovery yield or historical DMDE parity.

Synthetic tests additionally cover renamed originals, edited/unowned files, duplicate loss, interrupted cleanup, changed replacement bytes, conflicting destinations, stable collision mappings, catalogued acquisition identity and tampered history. No hardware read is needed for this checkpoint.
