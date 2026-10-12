# Saved recovery investigations

## Beginner: what survived?

From your project folder, while processing is idle:

```powershell
fv recovery investigate all
fv recovery investigate 133
```

This reviews existing captures, not physical floppies. It does not extract, convert, create reports inside the project, change a preferred image, choose disputed bytes, or update delivery. No Greaseweazle/Office process is launched. Operator output is English.

The batch view classifies the union of known live files with missing/disputed-sector dependencies and incomplete/ambiguous allocation chains. A file in both groups appears once. Each selected record explains its chain/hole/tail and supported Word/RTF text outcome. Filesystem errors, directory gaps and unlocated records remain in the underlying full impact report; no known file attribution is not proof of harmless damage.

If a Word body survives, this means its declared **main-text positions** survived the supported mapping checks, not that its original container, formatting, images, fields, other stories or meaning are complete. RTF results are only a bounded continuously readable prefix; the remainder's text length remains unknown. Unsupported formats retain raw evidence instead of guessed repairs.

To publish/reuse the existing automatic readable editions for one saved disk:

```powershell
fv recovery documents 33
# Optional English presentation, original text unchanged:
fv recovery documents 33 --language en
```

Open the printed HTML paths. The ordinary scan/process pipeline already applies these supported salvage engines when native recovery is needed. No manual DMDE extraction or document-text assembly is needed for this supported scope. [Recovery scope and limits](DEEP_RECOVERY.md).

## Advanced: exact source evidence

```powershell
fv recovery investigate 133 --json
fv recovery investigate 133 --attempt 2 --json
fv recovery investigate all --json
```

`--attempt` selects an Images acquisition, not a raw capture, and is per-disk only. Default selection uses the same preferred/default saved snapshot as `recovery impact`. JSON records normalized source paths plus original recorded names, source hashes, metadata/log/flux-provenance bindings, excluded sector maps, exact decoded extents/gaps, stream-role observations and explicit refusals.

An optional output redirection is an operator-owned copy, outside managed project outputs:

```powershell
fv recovery investigate all --json | Set-Content -Encoding utf8 '..\batch-investigation.json'
```

Confirmed cross-reader conflicts replay both recorded images/maps/confirmation. The result retains both sector hashes and individual differing bytes, plus FAT-copy observations and known allocation claims. It never chooses an original-content winner. Strict compound-file mapping can locate the affected stream/offset, but missing/disputed required metadata refuses attribution.

For Windows Word6 version `0x65`, a narrow FIB/PLCF probe maps declared CP positions to observed main-body byte ranges, including complex fast-save pieces. It deliberately does **not** decode an assumed code page or repair an editable document. Cross-reader comparison applies the selected GW file map to readable USB physical positions; this is not independent verification of the USB filesystem/name/allocation. Missing bytes suppress complete comparison hashes. The mapping layout is checked against the primary [LibreOffice reader](https://github.com/LibreOffice/core/blob/master/sw/source/filter/ww8/ww8scan.cxx); acceptance ceilings and refusals are FluxVault policy.

## Bounds and exit codes

- At most 256 affected file records per disk; larger results refuse, not truncate into a clean-looking report. Existing impact inventory bounds also apply.
- Existing document limits apply: 4 MiB parents, bounded metadata/allocation/read work and character/piece counts. Compound role inspection caps 256 streams / 16 MiB total stream input; required metadata is never read from placeholder bytes.
- Investigation output is capped at 64 MiB, including the full batch impact report. Word6 mapping permits at most 200,000 main positions / 4,096 pieces / 64 KiB CLX; malformed bounds, encrypted/platform-specific headers and metadata-as-text overlap refuse.
- Selected image/control snapshots are checked again before returning. Batch inventory changes are flagged; it remains per-disk verified snapshots, not an atomic snapshot of every artifact in the project. Rerun when idle if evidence changes.
- Exit **0**: no attention in this diagnostic scope. **3**: damaged/disputed/unknown evidence remains, including useful successful investigations. **2**: invalid input, inconsistent evidence, unavailable source or bounds refusal. A refused batch-impact source keeps an explicit raw impact row and attention/error status; other known cases remain distinguishable.

Deleted/carved/manual/converted-file attribution and arbitrary editable-document repair remain separate features. This command does not promise original-content certainty from a valid container or agreeing bytes.

Measured cases: [the 17 saved file records](../testing/DAMAGED_BATCH_CASES.md), [disk 133's two disagreements](../testing/DISK_133_REVIEW.md).
