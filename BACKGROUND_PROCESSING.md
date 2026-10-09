# Keep swapping; FluxVault processes the saved disks

New projects use background processing automatically. The ordinary command stays:

```powershell
fv scan --last-disk 136
```

After each verified acquisition, the disk enters a durable saved-image queue. Extraction, safe offline recovery, conversion, audit and workbook work continue while you swap/read the next disk. One separate worker losslessly packs raw captures. Neither worker opens a physical drive.

Report work now includes the full combined final audit, source inventory, current output hashes, actual delivery manifest and eight-sheet workbook. Immutable bundles and a complete-generation pointer protect report promotion; final reconciliation uses stronger checks for attention. [Report guide](FINAL_REPORTS.md).

Safe composites/mirrored-FAT results now flow into extraction automatically as catalogued **DERIVED** attempts. Their source/sector recipes are replayed before reuse; they remain attention evidence, not new clean physical reads. Unsupported publication is recorded without abandoning ordinary processing. [Handoff, restart and bounds](OFFLINE_RECOVERY_HANDOFF.md).

**Follow the scan's big swap banner.** Background progress goes to saved logs, not over your waiting prompt. `QUIT` or the endpoint ends feeding, drains saved work and reconciles the whole project. A completed partial disk stays partial; successful processing does not mean every original byte was recovered.

## Look at progress in another PowerShell window

```powershell
Set-Location 'C:\path\to\your\project'
fv processing status
fv status
```

`processing status` shows the recorded stage, pending/failed jobs, attention jobs and whether a processing owner is actually active. A stale stage after interruption is explicitly flagged. Add `--json` for source bindings/details. These commands do not check tools or access hardware.

While processing owns the project, another extraction/conversion/audit/report/package command is refused. Read-only status remains available. This prevents two processes from racing shared delivery files or reports. Use final reconciled reports after feeding/draining ends: intermediate project audits may include newly arrived images not processed yet.

## After interruption, without a floppy

```powershell
fv processing resume
fv storage resume
```

The first drains pending/failed image jobs and performs a final whole-project reconciliation, including a bounded explicit retry of failed conversion outputs. The second drains packing tasks. Source bindings are verified before processing; changed/missing evidence is preserved and reported, never accepted as new good data. Corrupt queue files are refused rather than silently discarded.

Alternatively, continue the same `fv scan` command: durable scan history re-enqueues verified completed results. Custody confirmation remains necessary for an interrupted physical read. Do not edit queues or reset numbering to recover from an error.

Closing/killing the process does not erase jobs. Graceful feeding completion drains them. On an acquisition error, already-running saved-file work can finish before exit; not-yet-started jobs remain durable. Immediate coordinated cancellation of all child tools is a separate acceptance item.

## Worker settings

```powershell
fv scan --conversion-workers 4
fv scan --processing-mode tail
fv scan --processing-mode background
fv processing resume --conversion-workers 4
```

- Four conversion jobs is the conservative default; requested values 1–16 are supported and saved for scans.
- Requested/effective worker counts are concurrency ceilings, not promises of that many running Office processes. Background scans retain their logical-CPU-minus-two ceiling; shared admission can reduce active jobs further under RAM/storage pressure.
- Files use a shared balanced queue, isolated Office profiles, bounded deadlines and integrity checks.
- New/hash-changed jobs are attempted normally. An unchanged failed job is not relaunched for every new disk. `processing resume`, `process` or `conversion retry` explicitly retries it later.
- `--processing-mode tail` retains processing after feeding only. Older journals retain tail mode until explicitly changed; new projects default to background. `--acquisition-only` skips downstream processing.

## Saved records

| Path | Purpose |
| --- | --- |
| `.fluxvault-processing/jobs/*.json` | Atomic image-hash-bound jobs; completed records retained for idempotent restart |
| `Reports/ProcessingStatus.json` | Atomic latest stage/outcome and recorded shared-resource budget |
| `Logs/ProcessingEvents.jsonl` | Stage timeline, elapsed time, disks, worker ceilings and resource/wait snapshots |
| `Logs/external-tools.jsonl` | Detailed extraction/conversion invocations |
| `.fluxvault-processing.lock` | One workstation writer; OS releases ownership on process exit |
| `.fluxvault-artifacts.lock` | Committed-image publication/report snapshot gate; not held during Office conversion |

Jobs arriving during a long conversion pass are coalesced into the next pass rather than launching one whole-project pipeline per disk. Image metadata is atomically published as the acquisition commit marker. Partial/private temporaries are not completed acquisitions. Existing images, originals and compatible project/report formats are preserved.

## Automatic resource admission

No extra policy file or command is needed. The controller shares one admission budget among Office conversion, saved-image recovery/extraction/audit and lossless packing:

- New background jobs share CPU slots. Foreground read/decode activity leaves two logical CPUs outside this allowance (one when idle), with a minimum of one background slot. Running tools finish normally; they are not suspended or killed to reclaim a slot.
- Windows available RAM is sampled before admission. Estimates reserve 512 MiB plus bounded source-size allowance per Office job, 256 MiB for saved recovery/audit and 64 MiB for streaming packing, plus 1 GiB free-memory headroom. Actual Office RAM/thread counts can differ.
- Packing and recovery/audit share one bulk-I/O slot. Packing waiters precede recovery, then Office; equal classes use FIFO admission. Office retains its balanced long/short job queue.
- Output and temporary locations on the **same Windows volume** add their estimated future storage reservations, including simultaneous USB/GW outputs. Each volume must retain 512 MiB free headroom. Read/decode preflight does not wait for background CPU slots and checks workstation destinations only, never source devices.
- Waits resample pressure and resume automatically when capacity returns. Continuous RAM/free-space pressure is bounded to 30 seconds; ordinary slot waits are bounded to 15 minutes. A refused job retains evidence/durable work and becomes attention, not a false Office timeout. After freeing resources use `fv processing resume` and/or `fv storage resume`. Wait time does not consume the Office host-process deadline.
- `fv processing status --json` exposes available RAM, estimated reservations, active/waiting counts, CPU slots and last deferral. Human status identifies these as **recorded** values; a separate status command does not invent live counters. An idle processing worker refreshes its snapshot every five seconds while its scan is still feeding.

These are admission estimates within **one controller process**, not hard OS memory quotas, CPU affinity, disk-bandwidth limits or cross-process scheduling. Destination probes currently require supported local Windows volumes and fail closed if unavailable. Reservations are deliberately conservative and may count already-allocated memory/disk bytes again. Existing project/device/capture locks still enforce ownership. Admission occurs before project snapshot/capture locks; the recovery permit is released before requesting Office permits. Full physical throughput/resource measurements remain a separate acceptance gate.

The processing timeline and its resource metrics stay internal and are excluded from customer ZIPs. Hash-bound acquisition/provenance records and customer reports retain their existing archival rules.

On Windows an explicitly selected `soffice.exe` uses its sibling `soffice.com` when available, preserving the selected installation and recording the actual executable in the audit. This avoids the GUI launcher's version-message console during background checks; custom wrappers remain unchanged.

Validation covers concurrent arrivals for 136 jobs, duplicate enqueue, interrupted processing, changed-source refusal/restoration, failed-work restart, ownership, stale status and bounded control publication. Saved customer evidence is tested separately from physical acquisition. Run the clean/DD/damaged checks in [PILOT_136.md](PILOT_136.md) before the full live cohort.

The real 20-disk pilot was copied into a separate validation project: all 172 conversion jobs succeeded across two coalesced processing runs, the final audit retained 17 verified disks and three attention disks, and every original image hash stayed unchanged. The fresh saved-evidence test took approximately 8 minutes 42 seconds including fixture copying and final checks; this is not physical scanning throughput. An executable scan test also exercised the default queue/drain/packing path with mock hardware and real saved-file tools.

Unrecognized automatic formats remain outside the image queue: FluxVault retains verified raw evidence and the format decision, shows an amber raw-only exception cue and continues feeding. They remain attention results in final status and cannot silently certify a customer package.
