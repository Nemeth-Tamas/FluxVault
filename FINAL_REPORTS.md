# Final report: what was saved, what passed, what needs attention

Normal scans and `fv process` automatically build the full report after saved-file processing. No Excel automation, audit script, or manual spreadsheet updates are needed for that workflow.

## Beginner: open the results

From your existing project folder:

```powershell
fv report export
# English presentation instead:
fv report export --language en
```

Open the printed **FloppyFinalReport.xlsx** path. Export checks saved images/files only: no floppy, Greaseweazle, extraction or LibreOffice operation. Hungarian is the default presentation; both languages use the same data and stable machine field/status names. Recorded forensic warnings remain in their recorded language.

| Sheet (English names) | Contents |
| --- | --- |
| Summary | Disk/image/file counts, conversions, integrity warnings and conversion chart. |
| Floppies | One row per saved image/known raw-format exception, with status and reasons. |
| Recovered Files | Selected whole-file candidates, actual SHA-256, method, source binding and delivery eligibility. |
| Conversions | Current checks alongside the previously recorded result. |
| Conversion Issues | Only partial/failed current results. |
| Integrity | Expected/actual generated hashes, source binding and basic container checks. |
| Delivery Files | Actual Converted files, hashes, roles and recorded provenance. |
| Scope | Limits and project-level issues; never a completeness certificate. |

Frozen headers, filters and colored statuses help navigate details. XLSX customer names are literal strings, never formulas. Very long cells are display-truncated with a notice; JSON/CSV retain complete values.

## Understand the status

`fv audit` now runs the combined checks and creates this report too. Its verified/attention counts and exit code use current delivery/output bindings; underlying EvidenceAudit files remain narrower compatibility outputs. It supports `fv stop`/Ctrl+C and needs no external tools.

| Status | Meaning |
| --- | --- |
| `OK` | Recorded image, selected files and current delivery/conversion artifacts passed scoped checks. |
| `PARTIAL: IMAGE READ` | Image/native/derived recovery retains completeness uncertainty; independently saved files may be useful. |
| `PARTIAL: CONVERSION` | One requested modern/PDF output passed; the other conversion did not succeed. |
| `CHECK: CONVERSION FAILED` | No requested output passed, or a formerly successful output changed/failed integrity. |
| `CHECK: NO RECOVERED FILES` | No selected recovered whole files were verified/inventoried; raw/fragments may still exist. |
| `CHECK: EVIDENCE` / `CHECK: REPORT STATE` | Missing/invalid source, delivery mapping, conversion state or unassigned delivery files prevent OK. |

All issues remain recorded even when one higher-priority status is shown. `OK` does not prove complete original content, document semantics, historical live/deleted ownership, or DMDE yield parity.

Legacy/manual recovery retains `MANUAL_UNVERIFIED` origin and disk-level attention. Its current file/output hashes can still pass conversion integrity independently; missing custody evidence is not falsely reported as an Office conversion failure.

Windows volume/recycle-bin and delivery-excluded metadata remain inventoried with `DeliveryEligible=false`; they do not trigger missing customer-file warnings. Signature/directory/fragment hypotheses retain their recovery labels. Raw fragments, Word text/HTML salvage and forensic deleted candidates are separate evidence, **not additional whole-file counts**.

## Advanced: saved exports and automation

Every complete export is an immutable generation:

```text
Reports/FinalReports/<generation>/
    FloppyFinalReport.xlsx
    FloppyFinalReport.json
    FloppyFinalAudit.csv
    FloppyFinalAudit.txt
    RecoveredFiles.csv
    ConversionResults.csv
    ConversionIssues.csv
    IntegrityValidation.csv
    DeliveryManifest.csv
    DeliveryManifest.sha256
```

`Reports/FinalReportLatest.json` points to the latest complete generation and binds all artifact hashes. It changes only after the bundle is saved/hashed; prior exports remain intact. Interrupted exports may leave `.partial` folders or a complete unselected generation, neither replacing the last committed report. Do not delete unrelated evidence to clean up exports.

Customer ZIPs include only the latest complete bundle and pointer, not stale/partial report history. Packaging verifies those bindings before selection and during ZIP streaming; a changed artifact blocks final promotion and preserves the partial archive.

The timestamped Hungarian acquisition workbook and `EvidenceAudit.json/.csv` remain available. Their narrower checks may differ from the combined final report; use the latter for current delivery/conversion status. Expected-output integrity rows include missing/failed requested outputs; `InvalidGeneratedOutputs` counts formerly successful outputs that now fail checks. No candidates means 0% success, not a claim conversions ran.

```powershell
fv report export --language en --json
```

JSON returns `workbook`, `acquisition_workbook`, `final_report.directory/json/latest`, disk/attention counts and `customer_delivery_certified=false`. Successful export returns 0 even when its data shows attention. Scan/process/finalize return attention code 3 when combined evidence requires it; `finalize` does not package an attention run.

CSV uses stable headers/statuses, UTF-8 BOM, quotes and multiline values. Formula-looking CSV cells receive a leading apostrophe for safe spreadsheet display; JSON retains exact values. Hash bookkeeping is not a cryptographic signature or completeness certificate.

Export owns the processing lock and artifact snapshot; run after feeding/draining. It refuses a competing owner and supports `fv stop`/Ctrl+C. It changes no original/extracted/converted payload. Direct expert extraction/conversion commands still need process/report refresh; all-command automatic refresh remains TODO.

## Script compatibility and saved validation

This replaces the useful final-audit metric/inventory/workbook layer of `New-FloppyFinalAudit_v1.2_Progress.ps1` in `G.zip`, without Excel COM. Familiar filenames/statuses have extra evidence fields: **not** identical legacy CSV schemas or a historical-yield certificate.

An isolated 001–019 pilot copy exported both languages and verified **814 original artifacts unchanged**. Counts: 182 source files (one excluded Windows metadata file), 507 delivery files, 164 conversion jobs. Current checks found **162 OK / two partial conversions**: disk 003's two DOCX hashes differ from recorded values, and two delivery originals are missing. Those discrepancies are reported, not silently adopted/replaced. Hashing does not diagnose why they changed.

The report marks 15 disks OK, three one-sector images `PARTIAL: IMAGE READ`, and 003 conversion/integrity attention. The earlier success CSV recorded 164 OK jobs; recorded success and current verification are deliberately separate. The original pilot/captures/journals and next disk 020 remain unchanged.

Retained replay: `C:\Users\User\Desktop\FluxVault-Test\Final-Report-001-019-20261009-v5`. The ignored regression uses `FV_REPORT_SOURCE` and a fresh `FV_REPORT_OUTPUT`; no physical media access. Full 136-disk yield comparison remains separate.
