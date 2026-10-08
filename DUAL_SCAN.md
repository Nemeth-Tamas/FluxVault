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
| `p`, `PAUSE` | Persistently stop new reads; active reads/background work finish |
| `r`, `RESUME` | Enable numbered confirmations again; starts no read itself |
| `q`, `QUIT` | Stop new reads; finish active reads and downstream work |

This pilot uses **one aggregator console**, with station-labelled progress and a heartbeat during quiet reads. No additional windows or competing independent scans are launched. Clean swap banners are green; partials/errors red. ASCII-first messages also work without color.

Both station views show an explicit `ACTION` line. A recurring ten-second refresh preserves swap/transfer instructions while the other reader is busy; `STATUS` refreshes immediately and includes elapsed read time plus the latest capture/decode message. The shared next fresh label is offered to either free station, never both at once. Live HD sector counts are provisional until automatic HD/DD identification finishes; a 720 KB disk can legitimately show `0/18` before its offline DD decode succeeds.

The current console uses **GW `N% Read pass (tracks)` / `N% Decode pass (tracks)`**, seeded from the launched supported command's exact cylinder/head range, not from sectors recovered. Unique visited tracks count once even during retries/backward order; each capture/decode/targeted reread resets its own meter. Unknown coverage says `working`, not a fabricated percentage. **100% tracks is not SAVED**: keep waiting for the green/red removal banner. The single-GW reading bar uses the same declared-range seed. Full host sector/flux details remain in `Logs/external-tools.jsonl`; default dual output keeps concise phase/percentage state, urgent diagnostics and blue ACTION cues, without repeating raw track messages, the command legend, empty transfer queues or lengthy ETA caveats on every heartbeat. Pace includes swaps/pauses; its rough ETA is fresh feeding only, excluding recovery transfers and file-processing tail.

### Mistyped or repeated numbers

`uN`/`gN` accepts the exact **NEXT FRESH** label, an available earlier USB partial on GW, or the station's interrupted/reserved exact-label retry. Already-completed numbers, skipped fresh numbers and labels outside this session's range are refused with **NO NEW READ**. Wrong input does not release the station's SAVED disk, change the journal or overwrite/reread its acquisition. To intentionally repeat a completed disk, use a fresh project rather than resetting a live dual cursor. A queued earlier USB partial is recovery, not duplicate fresh scanning; follow its available `gN` transfer cue.

## USB partial -> GW

Both stations take **fresh disks**. USB uses a fast first pass with **zero retry passes**, saves the partial image/map, and says **SET ASIDE FOR GW**. It keeps taking fresh labels. GW uses existing automatic HD/DD detection and Fast/Normal/Recovery/Detective stages, immediately recovering its own errors.

When GW is free, physically insert the earlier USB partial and type `g22`, for example. If 022 is still recorded SAVED in USB, this also confirms its transfer out of USB; no extra OUT is required. Otherwise it selects its set-aside queue entry. USB can keep feeding while GW recovers 022. Do not swap GW's current READING disk.

Before publishing a transfer image into `Images`, USB source image/metadata/log seals are checked again, geometry must match, and **all mutually readable sectors must agree**, with at least one shared readable sector. Failure preserves raw evidence but publishes no candidate image and does not complete the queue item. Consistency supports identity; it cannot prove a physical label. Missing text is not invented.

A damaged boot sector no longer prevents USB geometry/protection probing. If USB cannot produce a completed image at all, its identity remains interrupted rather than pretending partial success. A failed station asks for a same-label reseat/retry; the other remains usable.

## Resume and results

```powershell
fv scan --double --write-blocker-verified
fv production status
fv production benchmark
fv processing status
```

USB/GW selectors and endpoint persist; changing them on resume is refused. Defaults are Windows **A:** and GW **B**; first-session `--drive LETTER:` / `--gw-drive A|B` override them. Dual custody and ordinary project cursor are distinct: resume dual mode for held/interrupted disks, not ordinary scan.

Background extraction/conversion/audit/reporting and managed capture packing run while feeding. `--conversion-workers 4` is the conservative default (1–16 accepted, background capped to leave CPU capacity). `--acquisition-only` skips the file-processing tools/pipeline, not read guards, evidence checks or packing. Recovery/format/storage expert overrides are not accepted in this first dual pilot; its defaults are automatic. Packaging remains the existing `finalize`/package workflow after checking reports.

`--write-blocker-verified` retains the existing USB hardware assertion. Positive Windows protection and plausible geometry are still required, checked again at the raw **read-only** open. No registry changes/source-write path are added. Device reservations exclude competing FluxVault USB/GW readers across projects. Coordinator and background processing share one project owner; physical reads/Office conversion do not hold the short publication gate.

`QUIT` is graceful draining, **not immediate cancellation**: active GW recovery can use its stage allowances. Interrupted custody requires reconfirmation; SAVED disks retain removal obligations. A completed USB image left before receipt commit can be adopted without opening a drive, and GW reuses verified completed recovery.

Windows now supervises the controller and per-operation external host trees before they run. A forced controller exit during mock dual capture stops both host/descendant, preserves partial custody/evidence, and reopens without duplicate publication. This covers host processes, not proof of physical motor idle or every journal cutpoint; wait for drive activity to settle and reconfirm custody after abrupt exit. Existing USB reads use controller-owned threads/handles rather than a separate host. [Supervision details](PROCESS_SUPERVISION.md). For the next larger mixed/damaged live cohort, use [053–075 instructions](NEXT_SCAN.md).

Need a break? Type `PAUSE` (`p`). It blocks new `uN`/`gN` commands before changing custody, but lets existing reads finish and background work continue. Wait for **SAVED** before removing a disk; `u out`/`g out` still work while paused. Type `RESUME` (`r`), then your next numbered read command when ready. Resume does not read anything automatically. The pause persists across exit/restart: reopen with the usual dual command, then explicitly `RESUME`. Older journals without a pause field start enabled, still requiring numbered confirmations. A final range with all removals/queues complete drains normally, even if paused. This is feeding pause, not in-flight capture cancellation or suspension of the file-processing workers.

Finished sessions save unique `Reports/DualScan-*.json` files: per-station read/decode timings, image hashes, missing counts, errors, final queue, processing and packing results. Schema 3 retains `feeding_elapsed_ms` (the event loop, including swaps/waits) and `session_elapsed_ms` (preflight through final processing, before report publication), and adds the durable benchmark summary/export paths. Older schema 1/2 reports remain unchanged. Read timings can overlap; their sum is not wall-clock throughput.

## Pace and saved timing reports

The recurring `PACE` line shows distinct saved labels/hour for **this invocation**, including swaps and pauses. A rough **fresh-feed ETA** appears after three distinct fresh saves when an endpoint is set and feeding is not paused. It includes pending initial reads. Re-reading a USB partial on GW never doubles that label; transfers of older disks do not supply fresh ETA samples. Remaining recovery transfers, file processing and packaging are not predicted. This is an observed-sample estimate, not a promised finish time; restart begins a new live sample.

```powershell
# Offline: inspect verified custody and export a new timing snapshot.
fv production benchmark
fv production benchmark --json
```

New dual scans sync events into `Logs/DualBenchmark/.fluxvault-dual-benchmark-*.jsonl`: configuration/build fingerprint, confirmed reader starts, saved sealed receipts, failures, PAUSE/RESUME, explicit OUT and completion. Finished runs automatically export JSON/CSV to `Reports/DualBenchmark`; the offline command creates additional uniquely named snapshots without touching media or invoking host tools. These private logs/reports stay outside customer packages. Existing `fv benchmark report` remains the **single-GW** report.

The summary separates verified labels, timed labels and per-station receipts, and reports reader busy **union** and simultaneous USB/GW interval time. Intervals include decode/publication, not only physical rotation. Completed invocation wall times include swaps/pauses/preflight/file tail and exclude gaps between invocations; the timing log starts after coordinator opening, so it differs slightly from the outer DualScan timer. An interrupted invocation retains its last durable elapsed lower bound but has **unknown total duration**, not an invented finish. A trailing incomplete line is flagged; malformed committed records or receipts disagreeing with verified custody are refused and preserved.

OUT counts are explicit confirmation commands, not measured physical touches; replacing a SAVED disk can confirm removal implicitly. Neither command counts nor mock reader rates prove human handling time, recovery yield or a six-hour acceptance. Older pilots have verified receipts but no new timing events; reports flag missing timings rather than synthesizing historical measurements. Run the next live cohort with the newly built executable to collect them.

`production status` now prints concise station/custody actions rather than the entire receipt JSON. `--json` retains detailed receipts and adds `usb_transfer_pending`, which includes saved partials still held in USB as well as set-aside ones. The original `usb_recovery_queue` remains removal-confirmed only. Offline inspection cannot prove that a recorded READING disk is actively reading; it tells you to check the original console or resume/reconfirm after it ends, never to blindly move it.

Offline preview: `fv scan --double --plan [--last-disk N]`. It opens no drives/tools and changes no settings. `production status --json` checks saved receipts without another reader. Existing archive partials are not automatically imported into the transfer queue.

## Validation boundary

### First physical acceptance: 007–010 (2026-10-08)

USB 007 and GW 008 ran concurrently, followed by USB 009 and GW 010. USB 009 saved a partial image with three missing sectors; `g9` transferred that identity and produced a clean 720 KB image. All four disks finished with zero missing sectors in their preferred images; extraction verified 37 forensic files and 29 Office conversion jobs succeeded. Five background runs completed with no worker/storage errors. The operator's console measured 6m55s for the entire session, including swaps and the 009 transfer, not an isolated reader benchmark.

The first final audit reported three false conversion warnings because canonical Windows output paths were written as absolute paths in the CSV. The corrected release resolves both sides before generating confined relative paths. Offline `fv process` on this same pilot now returns success: four verified / zero attention, all 58 Office/PDF outputs reused, and all saved image/capture/extraction/delivery bytes unchanged. The original dual-run report remains unchanged as historical evidence; the refreshed `Reports/EvidenceAudit.json` and workbook reflect the fix. No disk needs rescanning for this warning.

The saved cohort also passed release `finalize`: process/audit succeeded and the archival ZIP verified all 196 inventoried members. The approximately 46.9 MiB ZIP and SHA-256 sidecar are retained outside the repo under `FluxVault-Test\Dual-007-010-Delivery-20261008-v1`. This is archival integrity verification, not a claim that every document's formatting/content or the entire historical customer recovery is certified.

Routine tests cover the event pump with overlapping mocked readers, USB continuing while GW is busy, earlier-label transfer, invalid input, removal, QUIT, station failure, receipt adoption, shared ownership and device exclusion. Subprocess tests run **mock GW only**, including real parent termination after a saved receipt and restart without rereading it. Guarded publication refuses mismatched candidates before creating an image; later acceptance reuses the capture and completed reuse checks the guard again. The earlier reopenable 136-label/14-transfer model still passes.

The live small-cohort result above validates concurrent hardware and one USB-to-GW transfer. It does not establish full-collection throughput/yield or six-hour acceptance. Raw-only/unsupported formats preserve evidence and stop that station's identity without claiming an image. Broader cancellation, resource adaptation and automatic queue prioritization remain separate work.

### Larger physical cohort: 053–075 (2026-10-08)

23 labels / 29 saved station receipts completed in **31m23s**, about **44 saved labels/hour** including swaps/recovery/processing. Six earlier USB partials transferred to GW; preferred images on **059 and 066** retain five/four missing sectors. The historical session also records one USB protection refusal; it was not bypassed. The transcript's baseline comparison is **226/239 identical payloads**, seven changed/six missing. This is a mixed cohort, not a linear full-136 time/yield projection.

USB-to-GW extraction attempt changes exposed source-path-only conversion reuse: 59 already hash-bound conversions were incorrectly flagged unbound. The fix matches exact source bytes, disk/relative delivery identity, output paths and conversion formats/filters across attempts; bounded historical fallback checks successful recorded output hashes. Changed/unbound output remains preserved/refused. Offline replay of this saved project now has **168/168 successful conversion jobs**, 21 verified disks / two genuine partials, and all **1,333 existing evidence/payload artifacts unchanged**. No reread is needed; the original DualScan report stays historical, refreshed processing/audit reports show the correction.

## Later: a second USB station

One GW plus two independent USB drives is a reasonable extension, not supported by today's `--double` command. The coordinator currently has exactly `Usb`/`Greaseweazle` identities and one USB receipt per label. Generalize station IDs/configuration, per-device reservations and protection checks, shared-number claim/custody journals, interrupted resume, transfer routing and timing reports before adding a third reader. Preserve old two-station journals. Each USB partial must retain its own origin when moved to GW; all stations must share one project owner and downstream queue, not competing console scans. The additional drive may improve fresh-feed throughput, but GW recovery and conversion/storage can become bottlenecks, so benchmark rather than assuming proportional speed.
