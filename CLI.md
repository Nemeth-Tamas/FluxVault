# FluxVault CLI

For the first Greaseweazle/NEC drive hookup, follow [GREASEWEAZLE_PREFLIGHT.md](GREASEWEAZLE_PREFLIGHT.md) with a disposable protected floppy before using customer media.

Build with `cargo build --release`; the executable is `target\release\fluxvault.exe` on Windows. To install a copy and optionally add its directory to your user `PATH`, run `powershell -NoProfile -File .\scripts\install-cli.ps1 -AddToPath` from the repository root; omit `-AddToPath` to copy without changing PATH, or add `-WhatIf` to preview. Open a new terminal after a PATH change. Running `fluxvault` without arguments shows command help; the desktop GUI has been removed. The release executable and installer were previously checked from PowerShell/CMD and with a disposable directory. No installation or PATH change was performed in your user profile.

From a project folder (or any subfolder), for example:

```powershell
fluxvault status
fluxvault disk list
fluxvault disk show 7 --details
fluxvault recovery plan
fluxvault recovery queue
fluxvault recovery compare 7
fluxvault recovery backup 7
fluxvault recovery composite 7
fluxvault recovery fat 7
fluxvault tools check
fluxvault tools show
fluxvault greaseweazle preview
fluxvault greaseweazle info
fluxvault greaseweazle capture 7 --gw-drive A --source-write-protected
fluxvault greaseweazle decode 7
fluxvault greaseweazle status 7
fluxvault greaseweazle compare 7
fluxvault greaseweazle consensus 7
fluxvault greaseweazle plan 7
fluxvault extract all
fluxvault extract disk 7
fluxvault files manifest
fluxvault conversion plan
fluxvault conversion run
fluxvault conversion issues
fluxvault conversion retry
fluxvault conversion retry C:\path\to\Extracted\001\problem.rtf
fluxvault report export
fluxvault audit
fluxvault package build --destination C:\CustomerPackages
fluxvault finalize --destination C:\CustomerPackages
```

Use `--project C:\path\to\project` to select a project explicitly. Add `--json` to a command for machine-readable stdout (including structured errors); long-running progress goes to stderr. Exit code 0 means complete, 3 means attention/partial, and 2 means invalid input or an operation error. These codes will be refined as production automation is added.

`disk show N` summarizes saved attempts. Add `--details` for their hashes, bad-sector LBAs, retry counts, and evidence paths; JSON includes those fields without an extra flag. Neither view accesses the floppy drive.

`fluxvault drive list` enumerates removable drives without reading inserted media. `fluxvault drive probe --drive A:` opens only an enumerated drive read-only, reads at most the first 512 bytes, and reports geometry and the Windows write-protection result. A positive software result is **not proof that this USB adapter enforces physical write protection**. `fluxvault acquire --drive A: --disk N --retries 2 --write-blocker-verified` requires an operator hardware-protection assertion, a positive Windows protection report, and plausible floppy geometry; the imaging backend checks protection again when it opens the drive read-only. On 2026-09-26, Windows reported `protected` for customer floppy 007 and the CLI completed a read-only 1.44 MB acquisition. That attempt had one unresolved sector; all other 2,879 sectors matched the earlier clean archived image byte-for-byte. This validates the CLI read path, not the adapter's physical write-blocking behavior.

`fluxvault scan --drive A: --write-blocker-verified` runs a guided single-drive loop from the project's current disk number. After each physical swap, type `READ` to image the inserted disk or `QUIT` to stop; other input does not start a read. `--count N` caps the number of disks in that session, and `--retries N` sets the same bounded sector retries as `acquire`. A completed partial image advances numbering and appears in the recovery queue; a failed acquisition does not advance it. Scan prompts and progress use stderr, leaving final `--json` output machine-readable. The multi-disk loop itself has only been tested with synthetic acquisitions; the one-disk `acquire` path has been tested live as described above.

`fluxvault tools check` runs 7-Zip, LibreOffice, and Greaseweazle version checks and records executed commands in the project tool audit log (or the application audit log when no project is selected). `tools show`, `tools set NAME PATH`, and `tools clear NAME` manage per-user tool paths. `greaseweazle preview` prints safe raw-capture and file-to-file decode command examples without executing anything; `greaseweazle info` runs an audited, read-only device/firmware query. `recovery queue` shows unfinished cases; `recovery compare N`, `recovery backup N`, `recovery composite N`, and `recovery fat N` operate on saved evidence. `recovery import N --source DIR --dmde-log FILE` copies external DMDE results into guarded project recovery locations without overwriting an earlier import. `extract all` and `extract disk N` need 7-Zip, not LibreOffice. `files manifest` refreshes the recovered-file inventory. `conversion plan` builds delivery paths without LibreOffice; `conversion run` executes bounded, audited Office conversion. `conversion issues` reads saved exceptions. `conversion retry [SOURCE]` reloads the project-scoped conversion state after a restart, retries all saved issues or the selected source, and rejects changed source hashes or paths. `process` runs the existing-image recovery, extraction, conversion, audit, and workbook pipeline.

`tools check` confirms the **host program** is callable, not that a Greaseweazle board is attached. `greaseweazle info` now parses the device section and reports `device_status`/`ready` in JSON; an absent or unverified board returns attention exit code 3 even if `gw info` itself exits 0. Raw capture performs the same read-only info preflight before reserving an attempt or issuing `gw read`. This matters because upstream `gw info` explicitly prints `Device: Not found` and exits 0 in that case. [Greaseweazle info source](https://github.com/keirf/greaseweazle/blob/master/src/greaseweazle/tools/info.py)

The new Greaseweazle capture/decode commands are **hardware-independent groundwork, not yet live-validated**. After connecting and validating the board/drive with a disposable protected disk, `greaseweazle capture N --source-write-protected` runs only `gw read` with `--raw` and `--no-clobber`, preserving a numbered SCP attempt, SHA-256, metadata, and a command audit. The flag records your physical-tab confirmation; it is not proof of hardware write blocking. The format is inferred from a saved 1.44 MB or 720 KB USB attempt when possible; otherwise pass `--profile ibm.1440` or `--profile ibm.720`. `--gw-drive A` is the default; a straight PC ribbon cable may require `--gw-drive B` per the [Greaseweazle drive-select guide](https://github.com/keirf/greaseweazle/wiki/Drive-Select). `--revs N` accepts 1–10, default 3. These are Greaseweazle drive selectors, not Windows drive letters.

`greaseweazle decode N` uses the latest completed SCP attempt by default, or `--capture-attempt N`, and verifies the source hash before running file-to-file `gw convert`. Derived images are kept separately under `Flux/Derived`, with their own hash and provenance, and are **not promoted into Images or treated as clean sectors**. The decoder records any `Found X sectors of Y` summary reported by Greaseweazle. For standard IBM 80-cylinder profiles, it records exact missing LBAs only if the entire reported grid is present and consistent; otherwise the map stays unknown. This is still Greaseweazle's report, not independent verification of each image sector. `greaseweazle status N` re-hashes saved raw and derived evidence, highlights missing/changed files, and shows reported sector counts/map availability without needing the board or host tool. Both decode and status return exit code 3 because sector-quality integration and delivery certification remain open. No physical drive is accessed for decode or status. `gw read --format` without `--raw` can regenerate flux rather than preserving what the disk emitted; FluxVault always pairs them for raw SCP captures, as documented by the [Greaseweazle image-type guide](https://github.com/keirf/greaseweazle/wiki/Supported-Image-Types).

`greaseweazle compare N` is an offline, read-only comparison of the best saved USB attempt and latest decoded flux image. It rechecks both image hashes and matching geometry, requires a complete Greaseweazle sector map, and reports sectors good in both but byte-disagreeing, USB-bad/flux-reported-good donor candidates, USB-only-good sectors, and still-unresolved sectors. Donor candidates are **not** automatically merged or certified; Greaseweazle's map is still vendor-reported evidence. A missing/inconsistent map, changed hash, unsafe path, or geometry mismatch stops comparison. No floppy drive or external tool is accessed.

`greaseweazle consensus N` compares the latest decodes from the two latest **distinct raw capture attempts**. It requires the same format, complete sector maps, intact raw and decoded hashes, and reports exact LBAs that agree byte-for-byte, conflict despite both being reported good, appear good in only one pass, or remain bad in both. Two decodes of one SCP do not count as independent captures. This is an evidence check, not a promoted composite or customer-delivery certification; it never accesses a floppy.

`greaseweazle plan N` combines the best saved USB attempt with decodes from the two latest distinct raw captures. It verifies paths, hashes, geometry, and complete reported sector maps before identifying USB-bad sectors where both flux images report a read and agree byte-for-byte. It separately lists one-flux-only sectors, unresolved sectors, and any USB/flux or flux/flux byte conflicts. Matching control sectors provide useful evidence but are not proof of physical disk identity. The command is offline and read-only, always returns attention code 3, and **does not create or promote a composite**.

Failed or interrupted offline decodes retain a numbered `.partial.json` attempt record (and any partial image). `greaseweazle status N` shows these as needing attention, and the next decode uses a new number instead of overwriting the failed attempt.

Office/PDF reuse requires both a matching saved source hash and a matching saved output hash, even after a restart. An older valid-looking output with no saved binding is preserved and reported as an issue rather than silently claimed as current conversion evidence. `conversion issues` reloads saved issues on every invocation.

`finalize --destination PATH` combines saved-image processing and package verification in one command. It requires at least one image and an existing destination outside the project. If recovery, conversion, or audit still needs attention, it reports that status and does **not** create a package. A successfully verified archival ZIP is still not a certification that every original customer byte was recovered.

The CLI now has a guided one-drive disk-change loop, but still lacks full zero-touch recovery policy, crash-safe production scheduling, and the two-drive workflow. Windows protection reporting has varied across test disks; the positive 007 result does not settle independent hardware write-protection validation. Track these in [TODO.md](TODO.md).
