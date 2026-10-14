# Completed capture/decode publication restart — 2026-10-14

Recovery now finishes a **prepared, hash-sealed host result** before considering another physical pass. This closes a window where an SCP or decoded image existed but its final metadata did not, so restarting could repeat already completed work.

## Publication and restart rules

The supported capture and decode adapters reserve a started receipt, run the host, validate its output, sync the workstation payload, atomically update/sync the prepared receipt, then publish payload and metadata without replacing existing files. Failed metadata updates preserve the previous receipt and the staged update. Unique update names prevent a retained update from colliding with the next one.

| Interruption point | Restart behavior |
| --- | --- |
| Started receipt reserved; host not finished | Keep the partial attempt. Do not claim success or reuse it as a donor. |
| Host returned; output not yet sealed | Keep unsealed output. It is not a certified completed result. |
| Prepared receipt sealed; payload not published | Check identity/settings, source and payload size/hash; finish the same slot offline. |
| Payload published; final metadata absent | Reverify published bytes and finish the same metadata slot offline. |
| Both published; recovery journal not yet updated | Reuse verified saved capture/decode through the existing journal reconciliation; no repeated physical read. |

Capture receipts bind exact profile, drive, revolutions, settings, filename and command. Decode receipts bind disk/capture/profile/geometry, original raw source hash and size, output extent/hash and recorded sector observations. Changed files, forged paths/extents, wrong settings, competing prepared decodes and conflicting destination bytes refuse publication. An existing destination is never replaced. Unknown/incomplete sector quality remains unverified; this path does not turn host claims into independent CRC or customer certification.

Prepared capture publication runs **before the stage-budget check**: an expired physical-read allowance cannot prevent finishing already completed saved work. A later missing decode can run offline; an already sealed decode is reused without running the host again. Ordinary explicit requests for a new capture or a new completed decode still create another immutable attempt. Read-only status inspection does not publish anything.

No source-media handles, board queries, writes, erase/clean/update operations or new physical-read authorization are introduced. Recovery is still invoked under the existing project/device/controller guards. Exact command binding conservatively refuses a prepared capture whose recorded absolute output path no longer matches the project; it does not silently reinterpret a moved receipt.

## Regression evidence

Six new routine tests cover both adapters' reserved, host-finished, prepared, payload-published and committed boundaries; project reopen; hash/settings/path/size/destination mutations; competing receipts; and end-to-end recovery with an expired stage budget. The offline-only backend panics on any `info` or `read` call. Completed cuts publish one image attempt, retain the reported missing sector, preserve raw bytes and reuse the result on a second invocation. Unsealed cuts preserve their evidence and do not become completed results.

Two saved USB/flux comparison/planning paths now explicitly refuse contradictory legacy log hashes before using their sector maps. The existing legacy snapshot regression covers both refusals; ordinary saved-file hash comparison remains distinct from readable-sector certification.

The full quality gate passes **782 routine tests / 33 opt-in tests**, with zero failures, formatting, locked all-target checking and strict Clippy. The local receipts are in the isolated replay project's `Logs/CapturePublicationQualityGate*.log`. Release SHA-256: `c3aa0edf8112343439b942b0780ea1bb5e44b58d28a09bd6bf4118060372d7fd`.

These are deterministic application-interruption tests using synthetic workstation artifacts, not newly performed physical reads or arbitrary workstation-power-loss qualification. All other acquisition-journal/cursor/USB/downstream publication boundaries and independent hardware acceptance remain separate requirements.

The TODO denominator and checked count remain **464/479**. No universal restart or every-boundary item is closed solely by this finite set of regressions.
