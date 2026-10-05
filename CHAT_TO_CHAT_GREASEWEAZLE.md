# Chat-to-chat handoff: Greaseweazle-only production mode

Date: 2026-10-05

This file is a handoff from the current ChatGPT session to the next development session.

## What the user wants

Build a **Greaseweazle-only workflow** for FluxVault that does **not require the USB floppy reader at all**.

The normal operator experience should be simple:

1. Insert floppy into the Greaseweazle-connected drive.
2. FluxVault performs a **fast first pass**.
3. If the disk is clean, finish immediately.
4. If not, automatically escalate through increasingly thorough recovery passes.
5. Re-read only the problematic areas when possible instead of repeatedly hammering the entire disk.
6. Stop when the disk is clean, when no further improvement occurs within policy, or when the configured recovery ceiling is reached.
7. Preserve raw evidence and provenance throughout.
8. The source floppy remains read-only.

The operator should not need to understand flux, sectors, retries, MFM, SCP, or recovery policy during normal use.

## Existing Greaseweazle groundwork already present

Current main already has the foundations:

- `greaseweazle info`
- immutable raw SCP `capture`
- offline `decode`
- `status`
- USB-vs-flux `compare`
- two-independent-capture `consensus`
- `greaseweazle plan`
- SHA-256/provenance checks
- read-only Greaseweazle command allowlist
- no `gw write`, erase, or clean path
- bounded/observable external Greaseweazle process runner
- mock-backed tests

Read these first:

- `TODO.md`
- `CLI.md`
- `GREASEWEAZLE_PREFLIGHT.md`
- `src/greaseweazle.rs`
- `src/flux_capture.rs`
- `src/cli/flux.rs`

Do not throw away those safety/provenance guarantees.

## Desired Greaseweazle-only recovery pipeline

Target behavior:

```text
INSERT DISK
    |
    v
FAST PASS
  raw capture + quick decode
    |
    +-- clean ----------------------> DONE
    |
    v
NORMAL PASS
  more revolutions / normal retry policy
    |
    +-- clean ----------------------> DONE
    |
    v
SECOND INDEPENDENT CAPTURE
    |
    v
CONSENSUS / SECTOR MAP
    |
    +-- all recovered --------------> DONE
    |
    v
TARGETED DETECTIVE MODE
  reread only unresolved tracks/cylinders
  progressively stronger settings
    |
    +-- improvement --> repeat bounded escalation
    |
    +-- no improvement / ceiling --> PARTIAL or UNRECOVERABLE WITHIN POLICY
```

## Suggested policy

Please design this as an explicit policy object/config rather than hard-coded ad-hoc commands.

Example stages:

### Stage 1: Fast pass

Goal: get clean/common disks out of the drive quickly.

- minimal sensible revolution count, e.g. 2
- whole-disk raw SCP capture
- immediate offline decode
- parse exact sector map
- if complete and internally consistent: mark the Greaseweazle acquisition clean and continue to extraction/conversion/audit
- preserve the raw SCP even when clean

### Stage 2: Normal pass

Only when Stage 1 has unresolved sectors.

- new **independent physical raw capture**
- e.g. 3-4 revolutions
- decode separately
- compare/consensus against Stage 1
- promote nothing silently; build a provenance-aware derived result

### Stage 3: Recovery pass

Only unresolved tracks/cylinders should be reread if Greaseweazle supports the required track selection safely through the existing runner.

- target tracks containing unresolved sectors
- increase revolutions, e.g. 5 then 8
- optionally use slower settle/step settings only if there is evidence they help
- retain every raw attempt immutably
- update confidence/provenance per sector

### Stage 4: Detective / stubborn-media mode

For the remaining bad sectors only.

Ideas to evaluate safely:

- multiple independent targeted captures
- forward/reverse track order if useful/supported
- increased revolution count up to a bounded maximum
- consensus across independent physical reads
- terminate if a pass yields no improvement
- configurable total-read/time ceiling so one disk cannot stall the batch forever

Do **not** repeatedly reread known-good tracks once a trustworthy sector map exists unless required for identity/control validation.

## Derived-image rules

The desired end state is a new derived sector image assembled from evidence, never modification of an original acquisition.

For each sector, store provenance such as:

```text
LBA 1267
source: greaseweazle_consensus
capture_attempts: [2, 3]
decode_attempts: [2, 3]
independent_reads: 2
byte_agreement: true
confidence: corroborated
```

Suggested acceptance hierarchy:

- two independent physical captures decode the same bytes -> strong/corroborated
- one capture only -> usable candidate but lower confidence
- conflicting independently-good bytes -> unresolved/conflict, never guess
- no valid decode -> unresolved
- recovered sector must never overwrite raw evidence

A Greaseweazle-reported good sector by itself is evidence, not magical truth. Keep the current conservative provenance model.

## Greaseweazle-only CLI/operator UX

Add a mode that does not depend on a USB acquisition existing.

Potential shape (names are suggestions, not requirements):

```powershell
fluxvault greaseweazle recover N --gw-drive B --profile ibm.1440 --source-write-protected
```

or ideally as part of eventual production mode:

```powershell
fluxvault production start --station greaseweazle
```

Normal output should be plain-language:

```text
Disk 007
Fast pass: 2876/2880 sectors
Normal pass: +3 recovered
Detective pass: cylinder 41 head 0
Remaining: 1 sector
Status: PARTIALLY RECOVERED
Next action: remove disk 007 and insert disk 008
```

Expert subcommands should remain available, but normal use should not require manually running capture/decode/consensus/plan in sequence.

## Hardware setup learned in the 2026-10-05 session

Board:

- Greaseweazle V4.1
- host tools 1.23
- firmware 1.6
- board enumerated correctly
- current shop cable is **straight**, so this setup uses **Greaseweazle drive B**
- observed spindle speed on the current drive was ~299.4 RPM

Current drive:

- NEC FD1231H
- exact P/N: `134-506791-322-4`
- 2005 hardware
- this particular drive appears faulty and should **not** be used to declare FluxVault live-validated

Observed fault:

- physical head 1 can decode valid IBM MFM and has reached 18/18 sectors on some tracks
- physical head 0 repeatedly produces flux but `ibm.scan` reports `IBM Empty`
- same pattern occurred across multiple Excel 5.0 Hungarian installer floppies
- no visible disk scratching
- no obvious mechanical damage/contamination found on inspection
- therefore suspect this specific drive's head-0/read path; obtain/test a healthy 3.5-inch PC floppy drive before declaring the FluxVault Greaseweazle path physically validated

Do not "fix" this by weakening validation rules.

## Immediate development goal for next chat

Once a healthy floppy drive is available:

1. Run the existing real-hardware preflight through **FluxVault itself**, not just direct `gw.exe`.
2. Confirm:
   - `tools check`
   - `greaseweazle info`
   - real raw SCP capture
   - offline decode
   - status
   - second independent capture
   - consensus
3. Update docs/tests to mark only what truly passed on hardware.
4. Then implement the **Greaseweazle-only automatic escalating recovery mode** described above.
5. Keep it read-only, resumable, immutable, provenance-heavy, and operator-simple.

The intended product is a floppy-eating archival appliance: insert disk, let FluxVault decide how hard it needs to try, and only bother the operator when physical disk handling is required.
