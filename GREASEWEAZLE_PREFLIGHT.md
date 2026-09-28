# Greaseweazle arrival: read-only preflight

This checklist is for the NEC FD-1231H and a **disposable, physically write-protected** test floppy. Do not start with customer media. The NEC drive is a 3.5-inch 1.44/0.72 MB mechanism, but its model does **not** tell us the format of an inserted disk; select a profile from the saved USB image or explicitly verify it. [NEC product sheet](https://katalog.atcomp.cz/katalog/205100149/Floppy.pdf)

1. With power disconnected, connect the drive, ribbon cable, and power lead according to the exact Greaseweazle board revision and drive connector markings. Do not guess connector orientation. For a conventional PC twisted ribbon cable, the drive is commonly selected as Greaseweazle `A`; a straight PC cable may need `B`. These are Greaseweazle selectors, **not** Windows drive letters. [Greaseweazle drive-select guide](https://github.com/keirf/greaseweazle/wiki/Drive-Select)
2. With no floppy inserted, install/configure the official `gw` host tool, then run `fluxvault tools check --project C:\Users\User\Desktop\FluxVault-Test` and `fluxvault greaseweazle info --project C:\Users\User\Desktop\FluxVault-Test`. If tool discovery fails, use `fluxvault tools set greaseweazle C:\path\to\gw.exe` and repeat. `tools check` only confirms the host program; `greaseweazle info` must actually report the board as ready. Upstream `gw info` can exit zero while printing `Device: Not found`, so do not rely on its exit code alone. [Greaseweazle info source](https://github.com/keirf/greaseweazle/blob/master/src/greaseweazle/tools/info.py)
3. Insert only a disposable disk with its write-protect tab in the protected position. Confirm disk number and format independently. The following example records a new, numbered **raw SCP** capture under `FluxVault-Test\Flux` without overwriting earlier evidence:

   ```powershell
   fluxvault greaseweazle capture 1 --project C:\Users\User\Desktop\FluxVault-Test --profile ibm.1440 --gw-drive A --revs 3 --source-write-protected
   ```

4. After removing the floppy, perform only offline checks: `fluxvault greaseweazle decode 1 --project C:\Users\User\Desktop\FluxVault-Test`, then `greaseweazle status 1` and `greaseweazle compare 1` with the same `fluxvault` prefix and project argument. A second *physical* raw capture, if genuinely needed, enables `greaseweazle consensus 1`; two decodes of one capture do not count.

FluxVault permits only `gw info`, raw `gw read`, and file-to-file `gw convert`; it has no Greaseweazle write/erase/clean path. Raw capture pairs `--format` with `--raw`, because Greaseweazle documents that omitting `--raw` can regenerate flux rather than preserve the disk's physical emission. [Greaseweazle image-type guide](https://github.com/keirf/greaseweazle/wiki/Supported-Image-Types)

The `--source-write-protected` flag records the operator's tab check; it is **not** proof that the connected hardware cannot write. Capture/decoding and the NEC/GW combination have not yet been physically validated in FluxVault. Decoded sector images and comparison results remain unverified evidence, never automatically promoted into customer output.
