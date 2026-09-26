# FluxVault CLI (in progress)

Build with `cargo build --release`; the executable is `target\release\fluxvault.exe` on Windows. Add the directory containing that executable to your `PATH` to call `fluxvault` from any folder. Running it without arguments opens the GUI; `fluxvault --help` lists commands.

From a project folder (or any subfolder), for example:

```powershell
fluxvault status
fluxvault disk list
fluxvault recovery plan
fluxvault recovery compare 7
fluxvault recovery backup 7
fluxvault tools check
fluxvault extract all
fluxvault extract disk 7
fluxvault report export
fluxvault audit
fluxvault package build --destination C:\CustomerPackages
```

Use `--project C:\path\to\project` to select a project explicitly. Add `--json` to a command for machine-readable stdout; long-running progress goes to stderr. Exit code 0 means complete, 3 means attention/partial, and 2 means invalid input or an operation error. These codes will be refined as production automation is added.

`fluxvault drive list` enumerates removable drives without reading inserted media. `fluxvault drive probe --drive A:` opens only an enumerated drive read-only, reads at most the first 512 bytes, and reports geometry and the Windows write-protection result. A positive software result is **not proof that this USB adapter enforces physical write protection**. Its behavior remains unverified; do not insert customer media into it for testing. There is currently no CLI acquisition command.

`fluxvault tools check` uses the same 7-Zip, LibreOffice, and Greaseweazle version checks as the GUI, and records executed commands in the project tool audit log (or the application audit log when no project is selected). `recovery compare N` compares the two latest compatible saved attempts; `recovery backup N` creates or reuses an immutable pass-1 evidence copy for an incomplete attempt. `extract all` uses the GUI's batch extraction service and needs 7-Zip but not LibreOffice. `extract disk N` applies the same clean-image eligibility rules to one disk, preserves manual recovery, and creates or reuses a pass-1 recovery backup when extraction is unsafe or unsuccessful. `process` additionally runs the existing-image recovery, conversion, audit, and workbook pipeline, and needs both 7-Zip and LibreOffice. These commands operate on saved project images, not a physical drive.

The CLI does not yet cover drive acquisition, full recovery controls, selective conversion, or the two-drive production scheduler. Track those in [TODO.md](TODO.md).
