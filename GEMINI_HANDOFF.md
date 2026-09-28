# FluxVault handoff for Gemini — 2026-09-28

Start from the current `main` branch; the last development commit before this handoff was `21c7781`. The repository is `C:\Users\User\Desktop\randomprojectsillneverfinish\FluxVault`, and `C:\Users\User\Desktop\FluxVault-Test` is the existing, **non-disposable** test project/evidence workspace. Do not reset or recreate it. Read [TODO.md](TODO.md), [CLI.md](CLI.md), and [GREASEWEAZLE_PREFLIGHT.md](GREASEWEAZLE_PREFLIGHT.md) first, then inspect the code and `git status` rather than assuming this note is current.

## Product and safety contract

- This is a Windows-first, **CLI-only** Rust floppy-archiving tool. Do not restore the retired GUI. The desired everyday workflow is eventually `fluxvault production start`: the operator swaps disks while software handles scanning, retries, Greaseweazle escalation, extraction, conversion, audit, and packaging. That production scheduler is **not implemented**.
- Source customer floppies are strictly read-only. Never add USB writes, `gw write`/erase/clean, registry write-blocker toggles, or a destructive test on customer media. A CLI flag asserting a protected tab is not proof of physical write blocking. Use a known-good disposable disk for any write-protection experiment, with the user's explicit coordination.
- Preserve numbered acquisitions, hashes, logs, and earlier recovery attempts. Decoded flux sector images are unverified evidence under `Flux/Derived`, not clean replacements for `Images` or automatic customer output. Do not guess missing bytes and call them recovered.
- The root contains `G.zip` (legacy script reference) and `TextilMuzeum_Floppy_Archive_20260920_115210.zip` (archive reference). Both are intentionally ignored by `*.zip`; inspect them read-only if useful, but **do not commit ZIPs**. Use the project's established `git add .`, logical commits, and push tested checkpoints to the existing GitHub remote.

## What works now

- USB CLI acquisition/scanning, project discovery, attempt comparison/composite, partial FAT reconstruction, extraction, Office conversion, evidence audit, XLSX report, and guarded package/finalize commands exist. Exact usage and caveats are in [CLI.md](CLI.md). The CLI's `--json` mode uses exit `0` complete, `3` attention/partial, `2` error.
- Greaseweazle `info` distinguishes a connected board from `Device: Not found` even when upstream exits zero. Capture requires a connected board and an operator protected-tab assertion. `capture` stores immutable raw SCP (`gw read --raw --no-clobber`); `decode` operates only on saved SCP files. `status`, `compare` (USB versus flux), and `consensus` (two distinct raw captures) are offline evidence tools. They verify hashes/geometry and expose disagreements; none promote donor bytes. Failed decodes retain numbered partial records. Key modules: `src/greaseweazle.rs`, `src/flux_capture.rs`, `src/cli/flux.rs`.
- The most recent safety fix treats dangling Windows links as occupied output names and rejects non-regular output files. The targeted Windows symlink test ran and passed. The most recent full run passed 108 unit tests and 3 integration tests (7 unit and 1 integration ignored), plus `cargo check` and a release build. Re-run these after edits.

## Best next work

1. Before hardware arrives, finish a **bounded, observable Greaseweazle process runner**: stream progress to stderr while preserving complete stdout/stderr in the audit, record host-tool version with each run, handle interruption/timeouts without losing partial evidence, and test entirely with a mock executable. Keep the command allowlist read-only. This corresponds to open items in TODO §9.
2. When the board and NEC FD-1231H arrive, follow [GREASEWEAZLE_PREFLIGHT.md](GREASEWEAZLE_PREFLIGHT.md): identify exact wiring/drive select, run `info` with no disk, then use a disposable physically protected disk for the first raw read. Coordinate physical insertion/removal with the user. Do **not** claim live validation until an actual capture/decode/status cycle succeeds.
3. After that, prioritize evidence-backed USB/flux donor validation and immutable provenance-tracked compositing, then the automation-first dual-drive scheduler. A `gw` sector reported “found” by itself is not independent validation. Keep the broader script-replacement and zero-manual-work target in TODO in view.

Useful checks from the repo root: `cargo fmt`, `cargo check`, `cargo test`, `cargo build --release`, `git diff --check`, `git status`. Avoid touching physical drives during ordinary code/test work; synthetic project fixtures already cover most workflow logic.
