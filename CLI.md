# FluxVault CLI

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

`fluxvault tools check` runs 7-Zip, LibreOffice, and Greaseweazle version checks and records executed commands in the project tool audit log (or the application audit log when no project is selected). `tools show`, `tools set NAME PATH`, and `tools clear NAME` manage per-user tool paths. `greaseweazle preview` prints safe raw-capture and file-to-file decode command examples without executing anything; `greaseweazle info` runs an audited, read-only device/firmware query and reports unavailable hardware as attention. `recovery queue` shows unfinished cases; `recovery compare N`, `recovery backup N`, `recovery composite N`, and `recovery fat N` operate on saved evidence. `recovery import N --source DIR --dmde-log FILE` copies external DMDE results into guarded project recovery locations without overwriting an earlier import. `extract all` and `extract disk N` need 7-Zip, not LibreOffice. `files manifest` refreshes the recovered-file inventory. `conversion plan` builds delivery paths without LibreOffice; `conversion run` executes bounded, audited Office conversion. `conversion issues` reads saved exceptions. `conversion retry [SOURCE]` reloads the project-scoped conversion state after a restart, retries all saved issues or the selected source, and rejects changed source hashes or paths. `process` runs the existing-image recovery, extraction, conversion, audit, and workbook pipeline. Except for gated `acquire`/`scan` and an explicit `greaseweazle info`, commands operate on saved project evidence, not a physical drive.

Office/PDF reuse requires both a matching saved source hash and a matching saved output hash, even after a restart. An older valid-looking output with no saved binding is preserved and reported as an issue rather than silently claimed as current conversion evidence. `conversion issues` reloads saved issues on every invocation.

`finalize --destination PATH` combines saved-image processing and package verification in one command. It requires at least one image and an existing destination outside the project. If recovery, conversion, or audit still needs attention, it reports that status and does **not** create a package. A successfully verified archival ZIP is still not a certification that every original customer byte was recovered.

The CLI now has a guided one-drive disk-change loop, but still lacks full zero-touch recovery policy, crash-safe production scheduling, and the two-drive workflow. Windows protection reporting has varied across test disks; the positive 007 result does not settle independent hardware write-protection validation. Track these in [TODO.md](TODO.md).
