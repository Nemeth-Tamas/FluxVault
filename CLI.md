# FluxVault CLI (in progress)

Build with `cargo build --release`; the executable is `target\release\fluxvault.exe` on Windows. To install a copy and optionally add its directory to your user `PATH`, run `powershell -NoProfile -File .\scripts\install-cli.ps1 -AddToPath` from the repository root; omit `-AddToPath` to copy without changing PATH, or add `-WhatIf` to preview. Open a new terminal after a PATH change. Running `fluxvault` without arguments opens the GUI; `fluxvault --help` lists commands. The release executable was checked from PowerShell and CMD, and the installer was exercised with a disposable directory. No installation or PATH change was performed in your user profile.

From a project folder (or any subfolder), for example:

```powershell
fluxvault status
fluxvault disk list
fluxvault recovery plan
fluxvault recovery queue
fluxvault recovery compare 7
fluxvault recovery backup 7
fluxvault recovery composite 7
fluxvault recovery fat 7
fluxvault tools check
fluxvault tools show
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

`fluxvault drive list` enumerates removable drives without reading inserted media. `fluxvault drive probe --drive A:` opens only an enumerated drive read-only, reads at most the first 512 bytes, and reports geometry and the Windows write-protection result. A positive software result is **not proof that this USB adapter enforces physical write protection**. Its behavior remains unverified; do not insert customer media into it for testing. `fluxvault acquire --drive A: --disk N --retries 2 --write-blocker-verified` is gated: use it only after independent write-protection validation with a known-good disposable disk or a verified hardware write blocker. The CLI refuses acquisition without the flag, a positive protection report, and plausible floppy geometry; the imaging backend checks protection again when it opens the drive read-only. This command has not been live-tested on hardware.

`fluxvault scan --drive A: --write-blocker-verified` runs a guided single-drive loop from the project's current disk number. After each physical swap, type `READ` to image the inserted disk or `QUIT` to stop; other input does not start a read. `--count N` caps the number of disks in that session, and `--retries N` sets the same bounded sector retries as `acquire`. A completed partial image advances numbering and appears in the recovery queue; a failed acquisition does not advance it. Scan prompts and progress use stderr, leaving final `--json` output machine-readable. This loop has only been tested with synthetic acquisitions and must not be used on customer media until the hardware protection discrepancy is resolved.

`fluxvault tools check` uses the same 7-Zip, LibreOffice, and Greaseweazle version checks as the GUI, and records executed commands in the project tool audit log (or the application audit log when no project is selected). `tools show`, `tools set NAME PATH`, and `tools clear NAME` manage the same per-user tool paths as the GUI. `recovery queue` shows unfinished cases; `recovery compare N`, `recovery backup N`, `recovery composite N`, and `recovery fat N` use the GUI's saved-evidence services. `recovery import N --source DIR --dmde-log FILE` copies external DMDE results into guarded project recovery locations without overwriting an earlier import. `extract all` and `extract disk N` use the GUI's extraction rules and need 7-Zip, not LibreOffice. `files manifest` refreshes the recovered-file inventory. `conversion plan` builds delivery paths without LibreOffice; `conversion run` executes the same bounded, audited Office conversion as the GUI. `conversion issues` reads saved exceptions. `conversion retry [SOURCE]` reloads the project-scoped conversion state after a restart, retries all saved issues or the selected source, and rejects changed source hashes or paths. `process` runs the existing-image recovery, extraction, conversion, audit, and workbook pipeline. Except for gated `acquire`, these commands operate on saved project evidence, not a physical drive.

Office/PDF reuse requires both a matching saved source hash and a matching saved output hash, even after a restart. An older valid-looking output with no saved binding is preserved and reported as an issue rather than silently claimed as current conversion evidence. The GUI now reloads saved conversion issues when a project is reopened.

`finalize --destination PATH` combines saved-image processing and package verification in one command. It requires at least one image and an existing destination outside the project. If recovery, conversion, or audit still needs attention, it reports that status and does **not** create a package. A successfully verified archival ZIP is still not a certification that every original customer byte was recovered.

The CLI now has a guided one-drive disk-change loop, but still lacks full zero-touch recovery policy, crash-safe production scheduling, and the two-drive workflow. Live USB acquisition remains blocked in practice until the write-protection discrepancy is resolved. Track these in [TODO.md](TODO.md).
