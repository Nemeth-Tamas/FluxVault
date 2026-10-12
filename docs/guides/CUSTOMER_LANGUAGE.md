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
```

`production`, `process`, `finalize` and explicit `report export` remember the choice in the project. Finalization additionally binds it into its saved options. `package build --language en` selects the new archive's README language without rewriting previous exports; if the selected final report uses another language, it refuses and asks for a matching `report export`. Production/finalization create matching reports automatically. `package publish` copies the already verified snapshot exactly, not a translated/rebuilt substitute.

An optional remembered default is also available:

```powershell
fv project settings set customer-language en
fv project settings clear customer-language
```

Clearing restores Hungarian for new work, not for an already-bound production/finalization resume. Changes require exclusive project ownership. Values are only `hu` and `en`; invalid/duplicate flags are refused.

## Current coverage

- Package `README.txt`: complete Hungarian/default and explicit English versions, including partial/derived/forensic recovery warnings.
- Selected final workbook: language-selected sheet names, summary metrics, column labels and scope warning. Final audit text uses the same scope language.
- Automatic final-report refreshes: use the saved project choice rather than silently reverting to Hungarian.
- JSON/CSV keys, status identifiers, filenames, hashes, source document content and raw evidence/tool diagnostics: unchanged, not translated.
- Prior report generations, packages and recovery editions: preserved byte-for-byte; a new export/archive is a separate snapshot.

This is the first delivery-language slice. The broader polish item remains open for localized Word/RTF recovery editions, remaining human diagnostic prose and legacy acquisition-workbook language selection. Technical status codes/raw warnings are not presented as already fully translated. `--language` is not yet accepted by `recovery documents`.
