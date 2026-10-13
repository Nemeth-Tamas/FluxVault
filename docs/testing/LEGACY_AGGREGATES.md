# Historical aggregate reconciliation — 2026-10-13

Run from any project or the repository; this command reads the ZIP directly and writes only stdout. It never extracts customer files, modifies the ZIP, runs the old PowerShell scripts or opens media.

```powershell
fv benchmark legacy --baseline 'C:\Users\User\Desktop\randomprojectsillneverfinish\FluxVault\TextilMuzeum_Floppy_Archive_20260920_115210.zip' --json
```

## Results and different scopes

| Source/rule | Result |
| --- | --- |
| Actual saved images | 136 |
| Archived index alone | 126 rows: 88 OK / 38 PARTIAL |
| Supplied final-audit imaging precedence | 94 OK / 42 not OK |
| Independently parsed primary logs | 102 OK / 28 PARTIAL / six missing logs |
| Supplied v1.2 final-audit rules, including generated integrity | 83 fully OK / 53 attention |
| Diagnostic counterfactual omitting generated integrity | 86 fully OK / 50 attention |

The index lacks 001–006, 009, 030, 059 and 133. The script's `ArchiveIndex`-first / `ExtractionSummary`-fallback rule supplies six additional OK labels, reproducing 94/42. Manually recovered/no-status claims are not reclassified as imaging OK. The parsed logs measure a different source and do not replace historical CSV claims.

The replay reproduces `New-FloppyFinalAudit_v1.2_Progress.ps1` from `G.zip`, inspected as data, not executed. It counts recovered leaf filenames except `__*`, checks only delivery-eligible paths reported `ModernOK`/`PDFOK`, and applies image-missing, no-source, invalid-generated, failed-conversion, partial-conversion, imaging-not-OK, OK precedence. Its historical Office check is ZIP/member presence, and its PDF check is header/tail EOF presence—not modern semantic rendering. Ambiguous duplicate labels refuse rather than silently overwriting claims.

Nine archived generated DOCX members fail that old integrity rule. Three labels previously otherwise OK become attention: 003 (two members), 064 (one), 076 (one). Other invalid members occur on already-attention 021, 074 and 123, so do not change the final label total. JSON retains every exact member name, SHA-256 and rule.

The original `Reports/FloppyFinalReport.xlsx` is **not present** in this ZIP. The documented 86/50 matches the diagnostic counterfactual; that is not proof the workbook omitted a gate, ran earlier, or contained a bug. Its exact history remains unknown. Original claims, three log-hash discrepancies, 34 index-hash discrepancies and one log-extent discrepancy are preserved, not overwritten. No equal-classification or customer-integrity certificate follows.

## Verification

- Original ZIP SHA-256 remains `5a002153f77523fa73766d333e58e11a0edb4016080d8c1e5ce39dd707de0cf1` before/after replay.
- All 6,261 members / 683,329,461 expanded bytes pass CRC/SHA streaming verification. Existing 1,667 source records and 972 conversion rows remain unchanged.
- Synthetic ZIP replay separately tests index precedence, fallback/unknown statuses, excluded source metadata, invalid successfully-generated output, ignored failed leftovers, missing logs and counterfactual separation.
- Actual receipt: `C:\Users\User\Desktop\FluxVault-Test\Reconciliation-20261013-80d209f4\legacy-final-verified.json`.

Fresh same-evidence recovery yield, deleted/carved scope and full DMDE parity remain separate in [baseline comparison](BASELINE_COMPARISON.md).
