# v1.1 — faster feeding, fewer stalls

Stable v1 remains tagged `v1.0.0`. Development is **1.1.0-dev**, not a new stable release. Keep this update focused:

1. **Opt-in USB fast handoff:** stop at the first persistent sector failure after a small retry budget, save the partial with explicit unknown coverage, queue its label for GW and keep USB moving. Default off; USB-only stays best effort.
2. **Fewer operator stalls:** improve interrupted/wrong-label and insufficient-comparison-evidence handling, preserving custody, safe release/retry and original captures. Resolve the zero-readable-USB exception before calling fast mode production-ready.
3. **Clear queue/progress cues:** show failed reads separately from untouched sectors, the next action and queue state; reduce repetition and add focused restart regressions.

No rare-format expansion, speculative document-repair campaign, extra-drive modes or scheduler rewrite in this scope. The wider 479-item roadmap is not expanded to manufacture completion percentages.

## First implemented slice

In a **fresh dual-station development pilot**, add:

```powershell
fv production start --double --write-blocker-verified --last-disk 4 --usb-fast-handoff
```

Two immediate sector retries by default. Add `--usb-handoff-retries 0`, `1` or `2` to change that budget. `scan --double` and its `start` alias accept the same options. `production resume --write-blocker-verified` restores the saved policy; existing dual settings refuse silent policy changes. Use a new project to opt in.

The stable installed v1 executable does not have these flags. This checkpoint builds/tests development/debug code while leaving the stable optimized v1 executable/package in place. Do not replace an active scanner.

Clean disks keep efficient track reads. A failed track falls back to sectors; a retry-resolved sector lets reading continue. After a persistent failure, **no further device reads occur**. The source handle closes before hashing/publication. Only workstation output is padded to the expected image size. Untouched bytes remain unknown, never invented successful reads.

The existing durable publication/coordinator is reused. Only after saved evidence verifies does the console issue REMOVE / SET ASIDE FOR GW, distinguishing failed-read and **NOT ATTEMPTED** counts. Removal/transfer confirmation remains required; stations remain independent.

## Evidence and acceptance

Metadata adds bounded `usb_fast_handoff` schema 1 (retry budget, total sectors, attempted prefix, terminal failed LBA or null). Logs add `USB_HANDOFF_POLICY` and, if needed, a bounded suffix `UNREAD_RANGE`. Metadata/log identity, extent, hash, coverage and retries must replay before loading.

Historical `bad_sectors` / `bad_sector_count` remain **unavailable-byte masks**, including untouched placeholders—not a measured physical-damage count. Native extraction, composite donors and transfer checks exclude all unavailable bytes. USB pass inspection labels untouched rows `not_attempted_placeholder`, with no successful/attempted pass or direction. Audit/console explain the distinction; Hungarian customer summaries identify unknown coverage explicitly. Old settings/default-off behavior remain compatible. Do not downgrade a fast-handoff project to v1: its older log parser cannot interpret the new unknown ranges safely.

Deterministic read-only fixtures cover clean track reads; 0/1/2 retry bounds; retry resolution; first/last-sector failures; no access after terminal failure; exact readable prefix/placeholders; cancellation/output errors; tampered metadata/log refusal; safe extraction of an intact prefix file; settings restoration; exactly-once queue/removal; readable-prefix transfer matching; production option replay; and device-free executable previews/invalid modes. Mock production resumes without a USB read.

**Before stable v1.1:** complete a short qualified live USB + GW pilot and provide a non-stranding resolution when no mutually readable sectors exist. The current identity guard correctly refuses such a byte match and preserves evidence; it is not bypassed. Driver reads may still block: a retry budget cannot promise instantaneous handoff/cancellation.

Checkpoint verification: **870 routine tests passed, zero failures; 33 environment-dependent opt-ins excluded**. Formatting, locked all-target check, strict all-target lint and the PowerShell completion-engine suite pass. The optimized stable executable still reports `FluxVault 1.0.0` and retains SHA-256 `b66290980e9e50cea84d37021f08385bac1e13d8903d307852c2a363c4f82a47`; debug reports `FluxVault 1.1.0-dev`. Local logs: chat workspace `V1_1-20261018/QualityGate.log` and `Completion.log`. No physical media was used for these tests.
