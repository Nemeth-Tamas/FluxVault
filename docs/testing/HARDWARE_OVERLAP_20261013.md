# Protected GW reads with background work — 2026-10-13

Targeted acceptance on the working Mitsumi, Greaseweazle selector **B**, host tools 1.23 / board firmware 1.6. The operator confirmed each physical label and open write-protect hole separately. USB remained empty. All physical commands were read-only; no source-media writes or firmware changes occurred.

## Input ownership regression

The first isolated launch (`GW-Overlap-20261013-b37071b7`) failed its GW information probe before physical reading. Office hosts also timed out. The supported stop returned exit 130 after 253.5 seconds; the observer saw **zero physical read hosts**, not a completed hardware test. Its logs and queued failed work remain preserved.

Managed argv-driven hosts previously inherited the controller's operator input. An executable regression now explicitly requires host stdin EOF while the controller's input pipe stays open: it fails before the fix and succeeds afterward. The shared supervised launcher now gives every probe/GW/7-Zip/Office host closed stdin. Only FluxVault owns labels, swap commands and STOP. Typed stop, forced exit, process-tree cleanup and restart tests continue to pass.

A real saved Word document converted successfully after the fix in the same redirected-launch style. A fresh full overlap retry then completed all 18 saved disk-007 Office jobs successfully. This is evidence for the current launch/input fix, not a diagnosis of every historical host crash or a guarantee that every document converts.

Scan acquisition/verification failures also print their actual reason immediately, before background cleanup finishes. This changes visibility, not custody, automatic retry or evidence-verification rules.

## Measured cases

| Physical disk | Fresh isolated project | Result | Physical passes | Capture time | Entire observed command | Read/background overlap samples |
| --- | --- | --- | ---: | ---: | ---: | ---: |
| Disposable WinWord 1, recorded as 001 | `GW-Overlap-Fixed-20261013` | Partial: 2,878/2,880 sectors; LBA 16 and 24 missing; zero conflicts | 3 | 123.7 s | 142.2 s | 77 |
| Customer 007, HD | `GW-Clean007-20261013` | Complete: 2,880/2,880 sectors; zero conflicts | 1 | 97.0 s | 155.6 s | 39 |
| Customer 009, DD | `GW-DD009-20261013` | Automatic `ibm.720`: 1,440/1,440 sectors; zero conflicts | 1 | 97.7 s | 116.1 s | 41 |

WinWord stopped on the existing bounded no-improvement rule. Its three complete SCP captures were packed losslessly; `greaseweazle status 1 --json` reverified all original flux hashes. The first full pass and targeted attempts remain distinct evidence. No missing sector was guessed or promoted to readable.

The WinWord project's background job was an isolated copy of the already-saved customer 007 image/log, with only the copied metadata paths rebound. It was deliberately seeded queued for this acceptance fixture, not presented as a newly acquired 007. All 18 Office jobs succeeded, then saved-file processing and packing drained. Its peak sampled owned-process working set was **1,292,726,272 bytes**.

The clean-007 project similarly resumed an isolated saved customer-001 job. The fresh physical 007 image SHA-256 is `ca3831409d0ef217dc732078be99fe52ee211ddd9fc35f98cd0a22040b72ab77`, **identical to the earlier independently acquired USB image**. The final two-image project had 26 successful Office jobs, no conversion failures, and exit 0. Peak sampled owned-process working set was **1,548,431,360 bytes**. Its whole-command time includes converting the newly acquired 007 after the physical read, not just capture time.

The DD-009 project independently resumed another isolated customer-001 copy, completed eight Office jobs without failures, selected 720 KB automatically and returned exit 0. Peak sampled owned-process working set was **1,594,060,800 bytes**. Its capture was packed and the original flux hash reverified. The new 009 image SHA-256 is `b10cfe96525bb0aec3952c84c8d6df124b908fb170cd33ce94395dd90c4cc7c2`, identical to the earlier 136-batch image. The operator confirmed all three physical disks removed after their reads.

All six original customer-001/007 IMG/catalog/log source bindings were rehashed after testing and remained unchanged. Background copies stayed in their isolated projects; neither original catalog was updated.

The successful physical runs used the stdin-isolated release executable SHA-256 `7c47964f83f7c1fcd6ca8cfa26396bf1c3f6d005a90fc8a53ed34a53a5b3b52f`. The later immediate-error-display change was verified by acquisition/verification failure tests, not by causing an additional physical failure. No release executable was rebuilt while a physical controller was using it.

## Receipts and limits

Fresh projects live under `C:\Users\User\Desktop\FluxVault-Test`. Each contains:

- `Reports/ObservedHostRun.json` and `ObservedHostResources.csv`: sampled controller/descendant working sets, physical-read host presence, Office host presence and current saved-file stage.
- `Logs/ObservedScan.stdout.log` / `.stderr.log`: complete redirected CLI output.
- `Logs/external-tools.jsonl`: actual command arguments, success, durations and supervision.
- `Flux/Recovery/*_job.json`, packed flux/hash manifests, immutable image/provenance and processing-job receipts.

The observer nominally samples every 500 ms; CIM and scheduling add latency. Overlap counts are observations, not exact durations. Peak memory excludes unrelated processes and is a sampled working-set sum, not a hard RAM limit or whole-machine peak. Capture times come from the recovery journal; entire-command time includes preflight, queued work, recovery, reporting and packing. Concurrent workstation activity is not normalized, so these are acceptance samples, not a controlled speed comparison.

Operators confirmed drive idle/removal separately; process exit is not a mechanical sensor. Original customer-batch evidence is not rewritten. These representative cases strengthen the existing hardware/background requirement; they do **not** close its full 136-disk contention/CPU/yield/throughput acceptance or every power-loss/journal boundary. Checklist remains **464/479**, with no added boxes or weakened criteria.

The final `scripts/check.ps1` gate passed formatting, locked all-target checking, strict Clippy and **767 routine tests, zero failures, 33 opt-in tests not run by that gate**. Receipt: `FluxVault-Test/Layout-Checkpoint-20261013-0af9df49/HostInputQualityGate.log`. The routine suite ran concurrently with part of the DD sample; the observer excludes its processes from owned RAM, but workstation CPU contention is not normalized.

Final release build (including immediate failure visibility) succeeded after all physical checks returned. Executable SHA-256: `a6fed831c716c267ded7d74277bacf372f0a0ac0d0d2cd00ad10a220c01b1dcd`.
