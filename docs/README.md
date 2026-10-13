# FluxVault documentation

New here? Start with the [beginner tutorial](guides/TUTORIAL.md) or the [cheat sheet](guides/CHEATSHEET.md). The [project overview](../README.md) and [TODO](../TODO.md) stay in the repository root. The plain-text Windows manual stays at [FLUXVAULT-MANUAL.txt](../FLUXVAULT-MANUAL.txt).

Command examples use the repository or an acquisition project as their working directory, not this documentation folder. Moving these guides changes no commands or saved projects.

## Operator guides

- [Tutorial: setup, scanning, recovery and delivery](guides/TUTORIAL.md)
- [Cheat sheet: everyday commands](guides/CHEATSHEET.md)
- [CLI: complete advanced reference](guides/CLI.md)
- [Production: feed disks and finish automatically](guides/PRODUCTION_WORKFLOW.md)
- [Stop and resume](guides/STOP_RESUME.md)
- [USB + Greaseweazle dual scanning](guides/DUAL_SCAN.md)
- [Background processing and conversion workers](guides/BACKGROUND_PROCESSING.md)
- [Capture storage and lossless packing](guides/CAPTURE_STORAGE.md)
- [Finish and publish a batch](guides/FINALIZATION.md)
- [Understand the final reports](guides/FINAL_REPORTS.md)
- [Customer language and the English operator interface](guides/CUSTOMER_LANGUAGE.md)
- [Optional project settings](guides/PROJECT_SETTINGS.md)
- [Recovery policies](guides/POLICIES.md)
- [PowerShell Tab completion](guides/SHELL_COMPLETION.md)
- [Import an original script archive](guides/LEGACY_IMPORT.md)
- [Saved-batch visibility, performance and expert GW settings](guides/SAVED_BATCH_TOOLS.md)

## Recovery and evidence

- [Damaged filesystems](recovery/DAMAGED_FILESYSTEM_RECOVERY.md)
- [Carving and embedded-file salvage](recovery/CARVING_RECOVERY.md)
- [Deleted candidates and readable fragments](recovery/DELETED_AND_FRAGMENT_RECOVERY.md)
- [Deeper directory, fragmented-file and Word/RTF text recovery](recovery/DEEP_RECOVERY.md)
- [Damage impact and file dependencies](recovery/EVIDENCE_IMPACT.md)
- [Read-only damaged-file and dispute investigations](recovery/RECOVERY_INVESTIGATION.md)
- [Saved-flux diagnostics](recovery/FLUX_DIAGNOSTICS.md)
- [Saved-sector inspection](recovery/SECTOR_INSPECTION.md)
- [Improved images flowing into offline recovery](recovery/OFFLINE_RECOVERY_HANDOFF.md)
- [Generation selection and safe delivery maintenance](recovery/RECOVERY_SELECTION.md)

## Testing and measured results

The saved-batch review records results; pilot/runbook documents also retain earlier test recipes. They are not instructions to rescan an existing archive or run a physical test without checking the disk identity.

- [Measured 136-disk batch review](testing/BATCH_136_REVIEW.md)
- [All 17 damaged/ambiguous file records](testing/DAMAGED_BATCH_CASES.md)
- [Disk 133's exact USB/GW disagreement scope](testing/DISK_133_REVIEW.md)
- [Comparison with the original script/DMDE archive](testing/BASELINE_COMPARISON.md)
- [009/017/020/058 historical candidate and stream review](testing/LEGACY_CASE_REVIEW.md)
- [Single-GW readiness gates](testing/SINGLE_GW_READINESS.md)
- [Greaseweazle setup and recorded hardware validation](testing/GREASEWEAZLE_PREFLIGHT.md)
- [136-disk runbook](testing/PILOT_136.md)
- [20-disk pilot](testing/PILOT_20.md)
- [053–075 batch recipe](testing/NEXT_SCAN.md)
- [Hardware-test launcher and result collection](testing/TOMORROW_TEST.md)
- [2026-10-13 clean/DD/damaged/program and USB metadata checks](testing/HARDWARE_20261013.md)

## Development and historical handoffs

- [Current progress and coverage measures](development/PROGRESS.md)
- [Latest missing-boot/competing-layout checkpoint](recovery/LAYOUT_COMPARISON.md)
- [Latest saved-only checkpoint: editable text, GW publication and historical counts](development/OFFLINE_CHECKPOINT_463.md)
- [Latest four bounded checklist closures](development/CHECKLIST_462.md)
- [Previous eight closures](development/CHECKLIST_458.md)
- [Windows host-process supervision](development/PROCESS_SUPERVISION.md)
- Historical checkpoints: [85%](development/CHECKLIST_85.md), [90%](development/CHECKLIST_90.md)
- Historical handoffs: [Gemini](handoffs/GEMINI_HANDOFF.md), [Greaseweazle chat-to-chat](handoffs/CHAT_TO_CHAT_GREASEWEAZLE.md)

Handoffs/checkpoints preserve their original context. Use the current [TODO](../TODO.md), [progress](development/PROGRESS.md), repository state and CLI help for today's work, not an old handoff's next-step list.
