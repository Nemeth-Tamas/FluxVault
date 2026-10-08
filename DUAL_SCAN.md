# Two drives, one numbered batch — live pilot

Plain `fv scan` stays **GW-only**. `--double` is opt-in; USB-only remains `fv scan --usb --write-blocker-verified`.

## First small test: customer 007–010

Use a **new project**, not an existing pilot. The earlier GW cohort saved these four clean; 009 is 720 KB, the others 1.44 MB. Run the newly built executable directly to avoid an older installed `fv` copy:

```powershell
$fv = 'C:\Users\User\Desktop\randomprojectsillneverfinish\FluxVault\target\release\fluxvault.exe'
& $fv init 'C:\Users\User\Desktop\FluxVault-Test\Dual-007-010-Pilot-20261008'
cd 'C:\Users\User\Desktop\FluxVault-Test\Dual-007-010-Pilot-20261008'
& $fv disk select 7
& $fv scan --double --last-disk 10 --write-blocker-verified
```

If that folder exists, resume it rather than initializing/selecting again, or choose a fresh folder name.

1. Insert **007 into USB A:**, protection hole open, then type `u7`.
2. Insert **008 into the working GW/Mitsumi drive B**, hole open, then type `g8` while USB is reading.
3. After either station says **SAVED / REMOVE**, feed its next disk using the displayed **NEXT FRESH** label: e.g. `u9` or `g9`, depending on the free station. Then use the next label for 010. Numbering is shared, not fixed odd/even allocation.
4. Remove each station's last saved disk and type `u out` / `g out`. The finished range drains background work automatically. `QUIT` stops earlier and finishes active reads first.

Never move a disk marked **READING**. Every read command confirms station, exact label and open protection tab. Feeding the next label also confirms removal of that station's previous SAVED disk. Empty Enter never reads; dual rejects `--no-verify`.

## Short commands

| Input | Meaning |
| --- | --- |
| `u7` / `u 007` | USB read of 007 |
| `g8` / `g 008` | GW read of 008 |
| `g22` | Recover an available earlier USB partial 022 |
| `u out` / `g out` | Confirm removal of that station's SAVED disk |
| `s`, `?`, `STATUS` | Both stations, next fresh label and transfer queue |
| `q`, `QUIT` | Stop new reads; finish active reads and downstream work |

This pilot uses **one aggregator console**, with station-labelled progress and a heartbeat during quiet reads. No additional windows or competing independent scans are launched. Clean swap banners are green; partials/errors red. ASCII-first messages also work without color.

## USB partial -> GW

Both stations take **fresh disks**. USB uses a fast first pass with **zero retry passes**, saves the partial image/map, and says **SET ASIDE FOR GW**. It keeps taking fresh labels. GW uses existing automatic HD/DD detection and Fast/Normal/Recovery/Detective stages, immediately recovering its own errors.

When GW is free, physically insert the earlier USB partial and type `g22`, for example. If 022 is still recorded SAVED in USB, this also confirms its transfer out of USB; no extra OUT is required. Otherwise it selects its set-aside queue entry. USB can keep feeding while GW recovers 022. Do not swap GW's current READING disk.

Before publishing a transfer image into `Images`, USB source image/metadata/log seals are checked again, geometry must match, and **all mutually readable sectors must agree**, with at least one shared readable sector. Failure preserves raw evidence but publishes no candidate image and does not complete the queue item. Consistency supports identity; it cannot prove a physical label. Missing text is not invented.

A damaged boot sector no longer prevents USB geometry/protection probing. If USB cannot produce a completed image at all, its identity remains interrupted rather than pretending partial success. A failed station asks for a same-label reseat/retry; the other remains usable.

## Resume and results

```powershell
fv scan --double --write-blocker-verified
fv production status
fv processing status
```

USB/GW selectors and endpoint persist; changing them on resume is refused. Defaults are Windows **A:** and GW **B**; first-session `--drive LETTER:` / `--gw-drive A|B` override them. Dual custody and ordinary project cursor are distinct: resume dual mode for held/interrupted disks, not ordinary scan.

Background extraction/conversion/audit/reporting and managed capture packing run while feeding. `--conversion-workers 4` is the conservative default (1–16 accepted, background capped to leave CPU capacity). `--acquisition-only` skips the file-processing tools/pipeline, not read guards, evidence checks or packing. Recovery/format/storage expert overrides are not accepted in this first dual pilot; its defaults are automatic. Packaging remains the existing `finalize`/package workflow after checking reports.

`--write-blocker-verified` retains the existing USB hardware assertion. Positive Windows protection and plausible geometry are still required, checked again at the raw **read-only** open. No registry changes/source-write path are added. Device reservations exclude competing FluxVault USB/GW readers across projects. Coordinator and background processing share one project owner; physical reads/Office conversion do not hold the short publication gate.

`QUIT` is graceful draining, **not immediate cancellation**: active GW recovery can use its stage allowances. Interrupted custody requires reconfirmation; SAVED disks retain removal obligations. A completed USB image left before receipt commit can be adopted without opening a drive, and GW reuses verified completed recovery.

Finished sessions save unique `Reports/DualScan-*.json` files: per-station read/decode timings, image hashes, missing counts, errors, final queue, processing and packing results. Timings can overlap; their sum is not wall-clock throughput. Existing `benchmark report` remains the single-GW report, not a dual speed/yield claim.

Offline preview: `fv scan --double --plan [--last-disk N]`. It opens no drives/tools and changes no settings. `production status --json` checks saved receipts without another reader. Existing archive partials are not automatically imported into the transfer queue.

## Validation boundary

Routine tests cover the event pump with overlapping mocked readers, USB continuing while GW is busy, earlier-label transfer, invalid input, removal, QUIT, station failure, receipt adoption, shared ownership and device exclusion. Subprocess tests run **mock GW only**, including real parent termination after a saved receipt and restart without rereading it. Guarded publication refuses mismatched candidates before creating an image; later acceptance reuses the capture and completed reuse checks the guard again. The earlier reopenable 136-label/14-transfer model still passes.

These are not simultaneous physical reads, throughput/yield measurements or six-hour acceptance. First run 007–010, then a damaged USB-to-GW transfer. Raw-only/unsupported formats preserve evidence and stop that station's identity without claiming an image. Broader cancellation, resource adaptation and automatic queue prioritization remain separate work.
