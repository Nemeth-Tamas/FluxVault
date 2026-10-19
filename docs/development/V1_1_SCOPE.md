# v1.1 — faster feeding, fewer stalls

## Live prefix-handoff / full-chain pilot passed — 2026-10-19

Fresh fixture: `C:\Users\User\Desktop\FluxVault-Test\V1_1-Live-20261019-04e8f3e0`. Internal 001 = **disposable WinWord 1**, internal 002 = **customer 007** (not customer 002). Operator confirmed identities, open protection holes and both removals. All source access read-only; no code/stable release changed during testing. `LIVE_TEST_IDENTITIES.md` in the fixture records details.

- WinWord USB: **12.638 s**, 16 readable sectors; LBA 16 failed initial + two retries. **2,863 later sectors not attempted**, not measured physical damage.
- Same WinWord in GW B: **148.557 s**, three capture/decode stages, **2,879/2,880** sectors, LBA 24 still missing, no conflicts. Ordinary readable-prefix identity guard accepted; no override. GW acquisition attempt 003 follows a separate background-derived USB edition 002; prior evidence preserved.
- Clean 007 USB ran alongside GW: **69.437 s**, all 2,880 sectors, no early stop. SHA-256 `ca3831409d0ef217dc732078be99fe52ee211ddd9fc35f98cd0a22040b72ab77` exactly matches prior 007. Reader-interval overlap **69.494 s**, including decode/publication.
- WinWord extracted **22 files** with partial attention. 007 extracted **19 files**, **18/18** Office/PDF jobs successful. Final audit: one verified / one attention. Both OUT confirmations complete; queue empty, no held tickets, no scan/storage errors; three saved captures/decodes replay and packing drained.
- Offline `finish` returned expected attention exit **3**. Independent verification passed **23,612,175-byte ZIP**, customer index and **11** front reports, with zero writes/tools/media access. ZIP beside project.json: `V1_1-Live-20261019-04e8f3e0_20261019_115627_459.zip`, SHA-256 `eeb64bb26f5eae2c0e6698976bf41ae09e448777e140e0be2a204a5937fbf770`. Archival integrity is not complete-content certification.

Durable session `Reports/DualScan-1792403754475-30844.json`: **362.427 s** including operator waits/removals and processing tail, not a 136-disk throughput prediction. GW image SHA-256 `4031e5011e0b14dbba8ae30d1e38594c06d72bb869280cb0ead16143502049eb`. Debug executable SHA-256 `a76e9e570d4c29e626d347bc02f5534cdd63ca4f46ead31643a00cdb12c38677`; stable v1 hash remains unchanged below. QA logs: chat workspace `V1_1-20261019/LivePilot-Finish.json` and `LivePilot-PackageVerify.json`. This physical disk had a readable prefix, so **zero-readable custody-only exception remains mock-tested**, not physically reproduced here.

Stable v1 remains tagged `v1.0.0`. Development is **1.1.0-dev**, not a new stable release. Keep this update focused:

1. **Opt-in USB fast handoff:** stop at the first persistent sector failure after a small retry budget, save the partial with explicit unknown coverage, queue its label for GW and keep USB moving. Default off; USB-only stays best effort.
2. **Fewer operator stalls:** improve interrupted/wrong-label and insufficient-comparison-evidence handling, preserving custody, safe release/retry and original captures. The zero-readable-USB exception now has an explicit, attention-retaining resolution; live acceptance is still required before calling fast mode production-ready.
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

## Zero-readable USB: explicit label confirmation (2026-10-19)

If USB fails at the very first sector, its padded image contains **no readable source sectors**. A later GW image cannot be matched to those zeros. The normal consistency guard still refuses publication and holds the interrupted label; it does not pretend the zeros establish identity.

The console now gives two explicit choices:

- **Wrong label:** wait for drive idle, remove the mistaken floppy, then type `g release N` using the **interrupted number**, not the number you meant to insert. The original USB label stays queued. The rejected GW job/captures remain preserved and excluded from future donors. No read starts. Insert the intended floppy and type its eligible `gN`.
- **Definitely the same physical floppy:** check its label and type `g confirm N`, then `gN`. Confirmation alone starts no read. It binds an operator assertion to the exact original USB image/metadata/log and already-decoded GW candidate/maps; `gN` publishes that saved candidate without another physical pass. Repeating confirmation retains the original proof/timestamp.

The second choice is **only** available for a replay-verified fast-handoff USB acquisition that failed at sector zero. It is not a general override for sparse/disjoint comparisons, arbitrary disagreements or changed evidence. Existing minor-disagreement confirmation keeps its original thresholds.

The new assertion is **operator custody only**, not byte-verified identity, reconstructed bytes or content certification. Even a GW image with zero missing/conflicting sectors retains a partial/attention receipt and `OPERATOR_CUSTODY_ONLY` audit status. English operator messages, immutable provenance and Hungarian-by-default customer warnings say so. Removing the warning from metadata or its receipt is refused on replay. Original source evidence remains unchanged.

Mock executable tests exercise initial refusal, wrong-number confirmation/release refusal, idempotent confirmation, exit/restart, publication while new capture/decode commands are disabled, attention after OUT, benchmark/audit replay, source preservation and warning-stripping refusal. A separate wrong-label run archives the rejected job, leaves its USB label queued and lets the station process another disk. These tests open **no physical devices**.

**Live prefix-handoff/full-chain pilot completed above.** Zero-readable exception/removal behavior remains mock-tested, not physically reproduced in that pilot. Driver reads may still block: a retry budget cannot promise instantaneous handoff/cancellation. Stable v1 remains untouched; this is not a stable v1.1 release.

Previous checkpoint verification (2026-10-18): **870 routine tests passed, zero failures; 33 environment-dependent opt-ins excluded**. Formatting, locked all-target check, strict all-target lint and the PowerShell completion-engine suite passed. The optimized stable executable reports `FluxVault 1.0.0` and retains SHA-256 `b66290980e9e50cea84d37021f08385bac1e13d8903d307852c2a363c4f82a47`; debug reports `FluxVault 1.1.0-dev`. Local logs: chat workspace `V1_1-20261018/QualityGate.log` and `Completion.log`. No physical media was used for these tests.

Current checkpoint verification (2026-10-19): **878 routine tests passed, zero failures; 33 environment-dependent opt-ins excluded**. Full formatting/locked all-target compilation/strict lint/test gates and PowerShell completion checks passed. Stable v1 executable/version/hash above remain unchanged. The development `package verify` also verified the existing 136-disk ZIP, customer index and all 11 front reports with **zero files written, no host tools and no physical-media access**; archive SHA-256 remains `5183f452b4aba04a7d2a8fadd0e928bd707106c2e8159771d8aa012ade050273`. This checks published snapshot integrity, not recovered-content completeness or current-project reprocessing. Local logs: chat workspace `V1_1-20261019/QualityGate.log`, `Completion.log` and `Customer136-PackageVerify.json`. The wider TODO remains **465/479**; this focused post-v1 change does not add checklist boxes.
