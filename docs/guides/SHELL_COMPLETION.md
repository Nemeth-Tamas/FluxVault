# Less typing: PowerShell Tab completion

Optional **PowerShell 7+** completion works with `fv`, `fv.exe`, `fluxvault`, `fluxvault.exe`, and the full executable path. It suggests current commands, context-specific flags and fixed values. It does not change scanning or confirmation behavior.

## Enable in this terminal

Without installing anything, paste:

```powershell
. 'C:\Users\User\Desktop\randomprojectsillneverfinish\FluxVault\scripts\FluxVault.Completion.ps1'
```

If you used the normal CLI installer, it now also copies the completion script. Refresh your installation, then use:

```powershell
. "$env:LOCALAPPDATA\Programs\FluxVault\FluxVault.Completion.ps1"
```

Custom installation folders: use the exact command printed by the installer. Loading is current-terminal only. To enable future PowerShell 7 sessions, optionally add that same dot-source line to your PowerShell 7 profile. The installer does **not** edit any profile or execution policy.

## Try it

Type a partial command and press Tab:

```text
fv sc<Tab>                         -> fv scan
fv scan --no<Tab>                  -> fv scan --no-verify
fv report export --language <Tab>  -> hu / en
fv scan --profile ibm.<Tab>        -> ibm.1440 / ibm.720
fv finalize re<Tab>                -> fv finalize resume
```

`<Tab>` means press the key; do not type those characters. More than one match cycles through choices. Numeric disk labels and device letters are not guessed. USB/dual mode filters hide incompatible ordinary-GW options; fixed capture/decode do not suggest `auto`.

The script uses static data and literal command text: it never invokes FluxVault, evaluates your arguments, reads project/customer files, enumerates drives, or probes media. Path-valued arguments get no custom suggestions; PowerShell itself may still offer its normal filesystem completion. Tab is not a swap confirmation and never starts a read.

## Advanced / testing

`fv completions powershell` prints the bundled script without needing a project or installed tools. You can save it to a trusted local `.ps1` and dot-source it. Windows PowerShell 5.1 prints a clear warning and does not register this optional native completer; the ordinary CLI still works there. Bash/Zsh are deferred until cross-platform support.

Developer checks: `cargo test --test cli_completion`, plus `cargo test --test cli_completion -- --ignored` for the actual PowerShell 7 engine. Static catalog checks run routinely. Engine tests cover full-path registration, aliases, mid-line cursors, quoted paths, fixed values, USB/dual filtering, and unevaluated subexpressions, with no physical media.
