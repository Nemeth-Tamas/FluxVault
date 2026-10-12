# Customer language, English operator interface

Scanning prompts, swap cues, progress, CLI help and the new language-validation errors stay English. New projects default to **Hungarian** customer delivery text. You do not need a flag for the normal Hungarian workflow.

## Everyday workflow

```powershell
fv production start --last-disk 136
```

For an English customer delivery:

```powershell
fv production start --last-disk 136 --language en
```

The production receipt saves the language separately from physical scan options. Resume normally:

```powershell
fv production resume
```

Resume restores the saved choice. A conflicting `--language` is refused before a new read; use a new project/finalization for a different customer-language snapshot. Legacy receipts without the field retain their previous Hungarian default.

## Saved-image finishing and expert commands

```powershell
fv process --language en
fv finalize --destination 'C:\Archives\English-Delivery' --allow-attention --language en
fv finalize resume
fv report export --language en
fv package build --destination 'C:\Archives\English-Delivery' --language en
fv recovery documents 22 --language en
fv recovery extract 128 --language hu
```

`production`, `process`, `finalize`, explicit `report export` and `recovery documents/extract` remember the choice in the project. Finalization additionally binds it into its saved options. `package build --language en` selects the new archive's README language without rewriting previous exports; if the selected final report or a newly language-labelled acquisition workbook uses another language, it refuses before creating an archive and asks for a matching `report export`. Older acquisition workbooks without a language label retain their archival compatibility and are not guessed/rewritten. Production/finalization create matching reports automatically. `package publish` copies the already verified snapshot exactly, not a translated/rebuilt substitute.

An optional remembered default is also available:

```powershell
fv project settings set customer-language en
fv project settings clear customer-language
```

Clearing restores Hungarian for new work, not for an already-bound production/finalization resume. Changes require exclusive project ownership. Values are only `hu` and `en`; invalid/duplicate flags are refused.

## Current coverage

- Package `README.txt`: complete Hungarian/default and explicit English versions, including partial/derived/forensic recovery warnings.
- Selected final workbook: language-selected sheet names, summary metrics, column labels and scope warning. Final audit text uses the same scope language.
- Acquisition workbook: the same saved language for sheet names, summary/header/chart labels, unknown timestamps and acquisition-only scope. Clean sectors do not certify original completeness or successful file extraction. Prior workbooks remain intact; each new filename includes language and a unique generation suffix.
- Final-report display shortening notices and raw-evidence explanations: localized. Raw technical issues retain their original evidence wording, paths/status codes and full JSON/CSV values.
- Automatic final-report refreshes: use the saved project choice rather than silently reverting to Hungarian.
- Word/RTF readable recovery HTML: Hungarian/default or explicit English labels, scope warnings, missing-text panels and parser-stop explanations. The recovered source text is never translated. Word engine 5 and RTF engine 3 publish separate `_hu` / `_en` folders and hash-verify reuse; earlier generations remain untouched. Raw text/JSON provenance keeps its original technical content.
- JSON/CSV keys, status identifiers, filenames, hashes, source document content and raw evidence/tool diagnostics: unchanged, not translated.
- Prior report generations, packages and recovery editions: preserved byte-for-byte; a new export/archive is a separate snapshot.

For an existing saved disk, run `fv recovery documents N` and open the printed HTML path. Use `--language en` for an additional English edition; switching back to `hu` verifies/reuses the Hungarian edition without replacing either. Attention exit 3 is normal: a translated presentation does not repair or certify the original document. To refresh an old completed package, create a new matching report/archive snapshot explicitly; old archives are not silently changed.

The broader polish item remains open for remaining customer-facing human diagnostic prose outside these reports/editions. Technical status codes/raw warnings are deliberately preserved, with an explicit original-evidence label rather than presented as fully translated.
