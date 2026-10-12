# Saved-flux diagnostics

**Inspect a difficult disk without inserting it or reading it again.**

## Beginner: one command

From the existing project folder:

```powershell
fv diagnose 59
```

Open the printed **Recovery note** for the short explanation. It shows the final missing/conflicting sector numbers and what changed between saved passes. The command also prints paths to the detailed JSON and track, committed-sector and CRC-observation CSV files, all under `Reports/FluxDiagnostics`.

No board, floppy, external tool, policy or format choice is needed. Raw and losslessly packed SCP captures work identically. This inspection does not select a different recovery image, change numbering, launch conversion or alter delivery. Run it after feeding/processing has finished: an active project owner blocks inspection rather than competing with acquisition.

Exit `0` means a replayed acquired result with no additional diagnostic issues. Exit `3` means partial/standalone evidence or diagnostic attention; it is not necessarily a crash. A changed committed capture, decode, image or provenance is refused with an error instead of being used as evidence.

## What the files show

| File | Useful question |
| --- | --- |
| `*-notes.txt` | Which sectors remain missing or disagree, and did a later pass help? |
| `*-tracks.csv` | Which captured heads/tracks/revolutions exist, how many transitions were recorded, and what was the index-period RPM? How many sectors were reported readable/unavailable on each decoded track? |
| `*-sectors.csv` | Which capture/decode attempts support each sector of the committed final image? How many different raw hashes support it? |
| `*-crc.csv` | Which IBM-MFM CHRN/size, ID/data CRC, clock violations, payload hashes and revolution/bit positions were independently observed? |
| `*.json` | Full revolution pulse measurements, capture availability, pass comparisons, independently replayed final provenance, source/export hashes and explicit unknowns. |

CSV head numbers are zero-based; sector numbers within a track are one-based. LBA is a zero-based sector index in the image. The JSON raw-track map includes uncaptured tracks explicitly. CSV raw rows contain measured revolutions, not synthetic rows for missing captures. Without a committed normal recovery, the final-sector CSV has only its header: standalone decoder output is not silently made authoritative.

### Read the distinctions correctly

- **Reported good** means the saved Greaseweazle decode map reports that sector readable. It is not a fresh independent CRC check by FluxVault.
- **Unavailable** combines missing sectors and bad-CRC results in the vendor map. Separate independent observations now distinguish ID/data CRC failures, missing data marks, per-revolution duplicate/unusual IDs and conflicting CRC-valid payloads where bounded IBM-MFM decoding succeeds. Unobserved regions are not failed observations.
- **Unobserved** means outside a targeted pass's cylinder range. It is not a newly failed sector. Different profiles have separate comparison histories.
- **Newly readable** identifies a previously unavailable sector first reported readable in a later saved decode. **Newly unavailable** compares only observed sectors with the preceding decode of the same profile. **Disagreeing reported-good bytes** identifies conflicts against an earlier reported-good value.
- Final-sector confidence and donor identities are replayed from the committed recovery job. Zero-filled unavailable sectors are never treated as observations. Multiple decodes/revolutions of the same raw capture are not independent physical reads; saved corroboration with fewer than two distinct raw hashes is flagged.
- Index-period RPM, transition counts and shortest/longest/mean pulse intervals are measurements. They do **not** prove weak bits, drive alignment, a correct file or repaired magnetic media. Failed/partial captures are listed but never promoted.
- Compositing/reconstructed filesystem bytes remain separate derived evidence with their existing provenance. This command reports the committed physical/decode recovery; it does not reinterpret a derived image as an original capture.

## Advanced

Independent saved-sector validation (no host tool or board):

```powershell
fv recovery crc-check 59
fv recovery crc-check all --json
fv recovery crc-composite 59
```

`crc-check` replays committed source binding and independently validates claimed good bytes against CRC-qualified payloads in their actual donor captures. Results distinguish validated, unconfirmed, conflicting and unresolved sectors; USB-only labels are not applicable. A missing sector remains missing. Code 3 means attention, including conservative unconfirmed observations; it does not prove bytes are wrong.

`crc-composite` is an optional advanced mutation, not a required production step. It admits original USB attempts plus the independently verified current GW catalog image, refuses inferred/unbound/unconfirmed/conflicting GW sources, and uses the existing good-byte conflict guard. The immutable composite gets a source/image/provenance-bound CRC sidecar. It is not automatically preferred or customer-certified. Changed published proof refuses; old composites remain preserved. Automatic ordinary composites retain their existing lineage validation, not a new blanket independent CRC certificate.

The bounded independent parser handles nominal 250/500-kbit IBM-MFM in checksum-valid supported SCP layouts. ID/data CRCs and ordinary clock checks qualify observations; deleted data marks remain recorded but are not ordinary donor evidence. Targeted captures, unsupported encoding/rates, absent data, limits and conflicting valid payloads stay explicit. CRC can verify bytes, not physical disk labels, allocation ownership, document semantics or magnetic weakness.

```powershell
fv diagnose 59 --json
fv diagnose 59 --project 'C:\Users\User\Desktop\FluxVault-Test\My-Batch'
```

`--json` returns the report/export locations, report SHA-256, counts and attention state on stdout. Resource-wait messages stay on stderr. The longer `fv greaseweazle diagnose 59` spelling is equivalent. Physical acquisition flags are rejected for this saved-only command.

To try a **different supported decoder profile**, not another physical read:

```powershell
fv greaseweazle decode 59 --capture-attempt 1 --profile ibm.720
fv diagnose 59
```

Use an alternate profile deliberately; it is not an instruction to relabel an HD image as DD. Offline decode requires the configured host tool but not the board; it creates another immutable, hash-bound decode under `Flux/Derived`. It does not replace the committed recovery or certify delivery. `diagnose` itself never invokes that tool.

### Integrity and bounds

Inspection reserves the project and shared background CPU/RAM/storage budget before work. Per disk: at most 64 capture records, 128 decode records and 256 KiB per immediate control file. Standard floppy SCP measurement supports the fixed 168-track table, 16-bit flux words, 1–10 revolutions and captures up to 512 MiB; ambiguous legacy single-sided/extended layouts are explicitly unmeasured. Raw measurement work is capped at 2 GiB per invocation; acquisition hashes and saved decodes remain inspected for skipped measurements.

Packed evidence is verified/materialized through the existing locked archive reader. Final image/provenance are confined to managed project directories and independently replayed; legacy optional stage fields remain compatible. Used controls/images, logical captures and the journal are rechecked before export. Unique, synced exports are read-back hashed; the JSON is published last without replacement as the commit marker. An interrupted export may leave uncommitted files but cannot replace an earlier report. Diagnostic working reports are excluded from customer packages.

The bounded SCP reader follows the [official SCP layout](https://www.cbmstuff.com/downloads/scp/scp_image_specs.txt), cross-checked against the [Greaseweazle SCP reader/writer](https://github.com/keirf/greaseweazle/blob/master/src/greaseweazle/image/scp.py). Index periods use fixed 25 ns units; the resolution multiplier applies to flux intervals. Overflow words carry across revolution boundaries and do not count as transitions.

## Retained validation

The saved live dual cohort's **059 and 066** were copied into `C:\Users\User\Desktop\FluxVault-Test\Flux-Diagnostics-059-066-20261009-v1`. Only absolute result locations in copied journals were rebound; raw/decoded/provenance payloads stayed unchanged. All **40 source artifacts** were hash-verified unchanged. All six packed captures measured with matching SCP checksums; legacy journals replayed to the original final hashes, five missing sectors on 059 and four on 066, with no byte conflicts. No new recovery yield is claimed by an inspection command.

Tests additionally cover malformed/aliased/overlapping/truncated SCP pointers, interval/index resolution, overflow boundaries, absent tracks, packed equivalence, targeted-pass coverage, alternate profiles, byte conflicts, failed/mapless records, owner/size refusal, changed data/provenance, redirected result paths, old optional stage fields, CLI flags/JSON and customer-package exclusion.
