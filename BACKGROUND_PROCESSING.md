# Keep swapping; FluxVault processes the saved disks

New projects use background processing automatically. The ordinary command stays:

```powershell
fv scan --last-disk 136
```

After each verified acquisition, the disk enters a durable saved-image queue. Extraction, safe offline recovery, conversion, audit and workbook work continue while you swap/read the next disk. One separate worker losslessly packs raw captures. Neither worker opens a physical drive.

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
- The background worker caps jobs to available logical CPUs minus two, minimum one, leaving capacity for acquisition/decoding. Status records requested/effective counts. This is a static CPU allowance, not adaptive I/O or memory budgeting.
- Files use a shared balanced queue, isolated Office profiles, bounded deadlines and integrity checks.
- New/hash-changed jobs are attempted normally. An unchanged failed job is not relaunched for every new disk. `processing resume`, `process` or `conversion retry` explicitly retries it later.
- `--processing-mode tail` retains processing after feeding only. Older journals retain tail mode until explicitly changed; new projects default to background. `--acquisition-only` skips downstream processing.

## Saved records

| Path | Purpose |
| --- | --- |
| `.fluxvault-processing/jobs/*.json` | Atomic image-hash-bound jobs; completed records retained for idempotent restart |
| `Reports/ProcessingStatus.json` | Atomic latest stage and outcome |
| `Logs/ProcessingEvents.jsonl` | Stage timeline, elapsed worker time, active disks and worker counts |
| `Logs/external-tools.jsonl` | Detailed extraction/conversion invocations |
| `.fluxvault-processing.lock` | One workstation writer; OS releases ownership on process exit |
| `.fluxvault-artifacts.lock` | Committed-image publication/report snapshot gate; not held during Office conversion |

Jobs arriving during a long conversion pass are coalesced into the next pass rather than launching one whole-project pipeline per disk. Image metadata is atomically published as the acquisition commit marker. Partial/private temporaries are not completed acquisitions. Existing images, originals and compatible project/report formats are preserved.

Validation covers concurrent arrivals for 136 jobs, duplicate enqueue, interrupted processing, changed-source refusal/restoration, failed-work restart, ownership, stale status and bounded control publication. Saved customer evidence is tested separately from physical acquisition. Run the clean/DD/damaged checks in [PILOT_136.md](PILOT_136.md) before the full live cohort.

The real 20-disk pilot was copied into a separate validation project: all 172 conversion jobs succeeded across two coalesced processing runs, the final audit retained 17 verified disks and three attention disks, and every original image hash stayed unchanged. The fresh saved-evidence test took approximately 8 minutes 42 seconds including fixture copying and final checks; this is not physical scanning throughput. An executable scan test also exercised the default queue/drain/packing path with mock hardware and real saved-file tools.

Unrecognized automatic formats remain outside the image queue: FluxVault retains verified raw evidence and the format decision, shows an amber raw-only exception cue and continues feeding. They remain attention results in final status and cannot silently certify a customer package.
