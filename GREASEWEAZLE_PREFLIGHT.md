# Greaseweazle: read-only setup and live validation

The tested setup is **Greaseweazle V4.1 + Mitsumi D353M3D-5056 (D63119) + straight ribbon + selector B**, host 1.23, firmware 1.6. The original NEC FD1231H showed a faulty head-0/read path and is not the validated drive. For a new/changed setup, start with an identified, physically protected test disk. Drive model does not determine disk format. Do not routinely swap ribbon connectors/drives as a recovery step.

1. With power disconnected, connect the drive, ribbon cable, and power lead according to the exact Greaseweazle board revision and drive connector markings. Do not guess connector orientation. For a conventional PC twisted ribbon cable, the drive is commonly selected as Greaseweazle `A`; a straight PC cable may need `B`. These are Greaseweazle selectors, **not** Windows drive letters. [Greaseweazle drive-select guide](https://github.com/keirf/greaseweazle/wiki/Drive-Select)
2. With no floppy inserted, install/configure the official `gw` host tool, then run `fluxvault tools check --project C:\Users\User\Desktop\FluxVault-Test` and `fluxvault greaseweazle info --project C:\Users\User\Desktop\FluxVault-Test`. If tool discovery fails, use `fluxvault tools set greaseweazle C:\path\to\gw.exe` and repeat. `tools check` only confirms the host program; `greaseweazle info` must actually report the board as ready. Upstream `gw info` can exit zero while printing `Device: Not found`, so do not rely on its exit code alone. [Greaseweazle info source](https://github.com/keirf/greaseweazle/blob/master/src/greaseweazle/tools/info.py)
3. Insert only a disposable disk with its write-protect tab in the protected position. Confirm disk number and format independently. The following example records a new, numbered **raw SCP** capture under `FluxVault-Test\Flux` without overwriting earlier evidence:

   ```powershell
   fluxvault greaseweazle capture 1 --project C:\path\to\Separate-Test-Project --profile ibm.1440 --gw-drive B --revs 3 --source-write-protected
   ```

4. After removing the floppy, perform only offline checks: `fluxvault greaseweazle decode 1 --project C:\Users\User\Desktop\FluxVault-Test`, then `greaseweazle status 1` and `greaseweazle compare 1` with the same `fluxvault` prefix and project argument. A second *physical* raw capture, if genuinely needed, enables `greaseweazle consensus 1`; two decodes of one capture do not count.

FluxVault permits only `gw info`, raw `gw read`, and file-to-file `gw convert`; it has no Greaseweazle write/erase/clean path. Raw capture pairs `--format` with `--raw`, because Greaseweazle documents that omitting `--raw` can regenerate flux rather than preserve the disk's physical emission. [Greaseweazle image-type guide](https://github.com/keirf/greaseweazle/wiki/Supported-Image-Types)

The `--source-write-protected` flag records the operator's tab check; it is **not** proof that hardware cannot write. Capture/decode/automatic targeted recovery have now been validated on the Mitsumi setup below. Expert decode/compare commands retain separate evidence; automatic `recover` can publish a derived image with explicit sector confidence and provenance, never customer-delivery certification.

Before the board arrived, an isolated end-to-end CLI test exercised tool setup, device info, refusal when the mock board is absent, two raw captures, offline decodes, status, consensus, and audit records. This checks the command wiring only; it does **not** substitute for the disposable-disk physical preflight above.

## FluxVault live checks — 2026-10-05

Disk: protected **WinWord 1**, confirmed by the operator. Project: `C:\Users\User\Desktop\FluxVault-Test\GW-Live-WinWord1`; disk 001 here is unrelated to earlier test/customer 001.

| Check | Observed result |
| --- | --- |
| Tools and board info | Host 1.23, firmware 1.6, V4.1; connected on COM9 |
| Full capture 1, 2 revolutions | Numbered raw SCP, hash and audit preserved; offline decode 2,878/2,880, missing LBAs 16 and 24 |
| Independent full capture 2, 3 revolutions | Offline decode 2,879/2,880; missing LBA 24 |
| Status and full-capture consensus | Intact hashes; 2,878 mutually good sectors agree byte-for-byte; sector 16 recovered only in the second capture; no good-sector conflicts |
| Automatic recovery seeded from capture 2 | Two targeted passes on cylinders 0, 1, 2, both heads; raw captures 3 and 4 preserved |
| Automatic result | 2,879 available sectors, LBA 24 unreadable, no conflicts; stopped after two non-improving passes |
| Job provenance | 107 corroborated sectors, 2,772 single-capture sectors, 1 explicitly unreadable sector |
| Repeat invocation | Zero physical reads; same published image/provenance rechecked and reused |
| Downstream processing | Immutable backup, evidence audit and Hungarian XLSX generated; partial image remains in recovery queue, no clean extraction claimed |

Published image: `Images\001_attempt_001.img` (1,474,560 bytes).

SHA-256: `4031e5011e0b14dbba8ae30d1e38594c06d72bb869280cb0ead16143502049eb`.

Provenance: `Flux\Recovery\001_attempt_001_provenance.json`.

The host compatibility fixes use `gw info`'s Host Tools version (not unsupported `--version`) and parse both stdout and stderr. No source-media write command was issued; the floppy was removed after physical testing. These checks do not validate every format/media condition, prove independent hardware write blocking, establish six-hour batch throughput, or complete damaged-filesystem extraction.

For routine **known 1.44 MB** disks, the new single-disk command is:

```powershell
fluxvault greaseweazle recover 7 --gw-drive B --source-write-protected
```

No USB scan is required. See [CHEATSHEET.md](CHEATSHEET.md) for the operator loop and [CLI.md](CLI.md) for policy/resume limits. A repeated completed job is reused, not physically reread. For a fresh hardware test, use a separate project or expert `capture`.

## Saved-image native recovery check — 2026-10-06

No floppy was inserted or read for these checks. The existing WinWord 1 derived image above was used unchanged.

| Check | Observed result |
| --- | --- |
| `recovery extract 1` | 22 complete reachable FAT12 files, 1,208,710 bytes; no skipped reachable entries |
| Source preservation | Image SHA-256 still `4031e5011e0b14dbba8ae30d1e38594c06d72bb869280cb0ead16143502049eb` |
| Native provenance | `Recovery/001/attempt_001_fat12.json`; per-file hashes, data/metadata LBAs and FAT-copy/chain evidence |
| `process` | Verified/reused native extraction; refreshed file manifest, mirrored delivery originals, audit and Hungarian XLSX; no Office candidates |
| Audit | 22 file hashes verified, `PARTIAL_VERIFIED_FILES`; disk still `PARTIAL_IMAGE_READ`, one unreadable sector, no customer-delivery certification |
| Independent file-byte cross-check | Fresh legacy-layout copy of the same image/log: all 22 file SHA-256 values and total bytes match independent 7-Zip extraction; DOS installer underscores such as `.EX_` preserved |

The parser reached an intact directory end marker before the bad root-directory sector (LBA 24); its report therefore has no traversed directory gap. The bad LBA remains explicit in acquisition/native evidence and audit. This recovers the currently reachable files, not proof that no deleted/orphaned files or other data were lost. No filenames/bytes were guessed, no existing extraction was replaced, and no new raw capture was requested.

The first-generation 22 files include 21 installer files and one 76-byte Windows `IndexerVolumeGuid` artifact, recovered under its recorded 8.3 path `SYSTEM~1/INDEXE~1`. At that checkpoint, validated long names and identifying OS metadata through those names were still future work. Do not interpret 22 as 22 customer-authored files. The comparison project is `C:\Users\User\Desktop\FluxVault-Test\Native-WinWord-Validation-b00fdb628f334dcdad274a15720cc1f7`; earlier development output is preserved separately in the original test project.

### Native long-name generation 2 — 2026-10-06

The saved image above was reused, with no physical drive access.

| Check | Observed result |
| --- | --- |
| Non-overwriting upgrade | Published `legacy_native_v2` and `attempt_000_fat12_v2.json`; first-generation extraction marker/report hashes unchanged |
| Original names | Two long-name associations validated; `System Volume Information/IndexerVolumeGuid` restored; no name fallbacks |
| Independent cross-check | All 22 relative paths **and** file SHA-256 values match 7-Zip extraction |
| Fresh project `process` | 22 forensic files, 21 installer delivery originals; Windows metadata not mirrored |
| Repeat processing | Verified reuse, 21 delivery-map rows, source SHA-256 unchanged, partial audit/workbook retained for LBA 24 |

Fresh processing project: `C:\Users\User\Desktop\FluxVault-Test\Native-LFN-WinWord-8ed96b6bdaca4de79c4e83906844c5d5`. Earlier delivery mirrors in older test projects were not deleted; safe automatic retirement of obsolete managed mirrors remains planned. Synthetic tests additionally cover Unicode names, malformed VFAT slots, alias/path collisions, fragmented directories, ASCII short-name case flags, inventory/provenance integrity, old-schema reuse and OS metadata exclusion from verified packages.

## Guided customer smoke checks — 2026-10-06

Protected customer disks were confirmed by the operator; each used a fresh isolated project under `FluxVault-Test` and one whole-disk physical capture. Both completed the guided scan, downstream processing, evidence audit, workbook and benchmark export.

| Disk | Actual profile / sectors | Observed recovery time | Downstream result |
| --- | --- | --- | --- |
| 007 | ibm.1440 / 2,880 of 2,880 | 104.790 seconds; 182.696-second complete session | 18 extracted files; all 18 file-content hashes match archived originals; 18 successful conversions, no evidence attention |
| 009 | ibm.720 / 1,440 of 1,440 | 101.760 seconds; 102.070-second complete session | One reachable FAT file, no eligible Office conversions; no evidence attention |

007 project: `C:\Users\User\Desktop\FluxVault-Test\Pilot-Smoke-007-20261006`; image SHA-256 `ca3831409d0ef217dc732078be99fe52ee211ddd9fc35f98cd0a22040b72ab77`. Its whole image differs from archived `Images/007.bin` at LBAs 6, 15, 22, 2007 and 2008; content-hash agreement does not mean byte-identical disk images or establish why these sectors differ.

009 project: `C:\Users\User\Desktop\FluxVault-Test\Pilot-Smoke-009-20261006`; image SHA-256 `b10cfe96525bb0aec3952c84c8d6df124b908fb170cd33ce94395dd90c4cc7c2`. This run used `--profile-map policies/customer-first-20-profiles.json` with the short three-pass/180-second policy, proving the DD override is actually wired into physical capture and decoding. The archive's 009 contains a damaged boot image and signature-carved outputs/recovery-tool reports; their hashes/counts are not equivalent to this reachable-file extraction. Carved/deleted-content comparison remains an explicit regression task.

These two observations are useful shakedown measurements, not a 20-disk or 136-disk throughput guarantee. The guided 20-disk cap, 008-HD/009-DD/010-HD switching, restart binding and downstream preflight also have synthetic/cross-process tests. Extraction now explicitly requests UTF-8 console output from 7-Zip; a saved-image integration check verifies Hungarian filenames in the listing and hash-bound extraction reuse without another physical read. See `PILOT_20.md` for the current attended-run command.
