# Project defaults and preferred images

Everything here is optional. The normal `fv init` / `fv production start --last-disk N` workflow still needs no custom policy file or tool configuration when tools are discoverable.

## Beginner: remember a worker default

Run from your project folder:

```powershell
fv project settings
fv project settings set conversion-workers 12
fv project settings set operator 'Archive team'
```

New runs use the worker default. Explicit `--conversion-workers` wins; an existing scan keeps its bound saved count until deliberately overridden. Four remains the conservative default: twelve simultaneous Office jobs are not a proven faster setting on every machine. These commands never read a floppy.

Restore the default with `fv project settings clear conversion-workers`. Settings are optional extensions in `project.json`; numbering preserves older producers' unknown fields. Changes require exclusive project ownership, use synced atomic metadata replacement, and refuse invalid counts, multiline operator names or unsafe executable paths.

## Advanced: project-specific tools

```powershell
fv project settings set sevenzip 'C:\Program Files\7-Zip\7z.exe'
fv project settings set libreoffice 'C:\Program Files\LibreOffice\program\soffice.com'
fv project settings set greaseweazle 'C:\Tools\Greaseweazle\gw.exe'
fv tools check
fv project settings --json
```

Use your actual existing absolute paths. These are executable paths, not a shell command or arguments. Project overrides take precedence over per-user `tools set` settings; clearing an override restores inheritance. Tool versions recorded by `tools check` are historical observations, not proof that tools/devices are ready now. Checking tools can probe the GW board; displaying settings never does. Old projects need no migration/rewrite just to open or inspect them.

## Preferred image attempts

```powershell
fv disk show 7 --details
fv disk prefer 7 2
fv recovery sector 7 --lba 0
fv disk prefer 7 auto
```

The number after the disk is an **image acquisition attempt**, not a raw-flux capture or decode number. Automatic evidence ranking remains the default. A pin selects one completed original or replay-verified DERIVED image and binds its image, log and metadata hashes. Older attempts remain immutable. The local `.fluxvault-preferred-images` folder retains selection history/failed partial controls; it is not physical-media custody and is excluded from customer archives like other private control data. Exported audit/report bundles explicitly identify the selected image/hash.

A missing, altered or ambiguous pinned attempt produces an error, never an invisible fallback. `auto` explicitly clears a stale pin when remaining acquisition evidence is valid. Selection is not a clean-disk certificate: choosing a partial/DERIVED attempt retains missing-sector/attention status. Unsupported or unbound legacy evidence cannot be pinned as if verified.

Native extraction, file inventory, integrity audit and the final workbook refresh automatically. Native recovery can preserve or refuse unsupported inputs rather than invent files. Existing operator/manual recovery remains preserved. Office is not launched by selection; use `fv process` to reconcile conversions if needed. Read-only sector diagnostics honor the pin by default; explicit `--attempt` is still available for comparison. No drive, registry setting or media write is involved.

## USB-only feeding

```powershell
fv scan --usb --write-blocker-verified --last-disk 20 --color auto
```

Only use the protection assertion after independent adapter/blocker validation. One forward pass is the default; saved partials are queued for GW rather than repeatedly stressing them with USB retries. Type the displayed label after each swap. `OUT`, `PAUSE`, `RESUME`, `STATUS`, `QUIT` and `STOP` are supported. Resume the same project and command; endpoint and completed evidence survive restart. `--acquisition-only` explicitly disables downstream processing; otherwise host-tool preflight runs before insertion and the saved-file queue processes between swaps.

After stopping USB feeding and confirming removals, `fv scan --double --write-blocker-verified` can work its retained queue using exact `gN` labels. It inherits the recorded endpoint. Do not launch a second competing scan. Blank Enter remains unavailable for USB/dual custody; `--no-verify` is single-GW only.
