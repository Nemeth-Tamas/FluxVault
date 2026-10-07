# Recovery policies, without homework

**You do not need to make a policy file.** A policy is simply the program's recovery budget: how many attempts it may make, how thorough they may be, and when it must stop. It prevents one stubborn floppy from consuming the whole afternoon or being reread indefinitely. FluxVault chooses whether to continue from the recorded results; you do not pick a pass at every disk.

## The normal command

```powershell
fv scan
```

In an existing scanned project, this reuses the saved drive, default format, per-disk format list, recovery policy and last-disk target. These are saved as values, so the original policy/map filenames do not need to be supplied again. Explicit expert flags override saved settings; incompatible changes are refused while a disk is pending.

Fresh defaults are Greaseweazle B, **automatic IBM 720 KB / 1.44 MB discovery**, packed retention, background saved-file processing, built-in recovery budget and no endpoint. `fv scan --last-disk 20` adds a stopping point. Inconclusive completed format trials become verified raw-only exceptions and advance custody without inventing geometry; tool/integrity failures still stop. Old journals retain fixed/raw/tail modes; between pending jobs, `--profile auto --capture-storage packed --processing-mode background` opts in. Fixed maps/profiles, raw storage and tail work remain expert choices.

Every physical prompt asks you to check the label and open write-protect hole. Type `004` (or `4`) to confirm the displayed disk, or `QUIT` to finish. Blank input and the wrong disk number start no read. Older `READ 004` input still works for existing scripts, but is unnecessary.

## Which budget am I using?

| Budget | What it allows |
| --- | --- |
| Built-in normal | Up to four escalating stages, **600 seconds of capture time per stage**, stop after two completed non-improving passes |
| Short example (`policies/pilot-short.json`) | Up to three stages, 180 seconds of capture time per stage, stop after two completed non-improving passes |

Both stop early when the reported sector map is complete. Later passes target unresolved areas plus clean identity-control cylinders. More retries do not guarantee more files. Missing/conflicting bytes remain flagged, never guessed. Raw captures are retained.

Fast, Normal, Recovery and Detective each have an independent allowance. Earlier stages do not consume Detective's ten minutes. A stubborn disk can therefore use up to approximately **40 minutes of physical capture time**, plus offline work; clean disks still stop after Fast. Failed/reseat attempts consume the same stage clock. Paused time between completed attempts is not charged, but an in-flight operation interrupted by a process crash is conservatively charged on restart rather than renewing its allowance.

If a later stage exhausts its allowance, its interrupted capture stays separate and FluxVault can move to the next configured stage using verified earlier evidence. At the end, unresolved sectors produce the normal red partial-saved swap banner. Unfinished flux is never decoded or sent to the completed-capture packer. Unexpected early tool timeouts, corrupted evidence and no completed usable pass still stop. Offline decoding has its own bounded timeout and does not use the next stage's physical-read allowance.

Policies without `time_limit_scope` now use `per_stage`, including existing saved scan settings. Unfinished older jobs are upgraded once when resumed, preserving completed passes, failed captures and original timestamps; the pending stage gets its new independent allowance. Already completed jobs are verified/reused, never reopened automatically. To explicitly retain the previous whole-disk budget, set `"time_limit_scope": "whole_job"` in a policy for a new job. Its historical empty-first-capture restart and partial-save behavior remain supported.

## Custom files are optional expert controls

Only if you want different limits, copy `policies/pilot-short.json`, edit your copy, and supply `--policy 'C:\path\to\my-policy.json'` when starting a new scan/job. Do not change a pending job's policy. Its saved values must match so a restart cannot silently change the plan.

| JSON field | Meaning |
| --- | --- |
| `passes` | Ordered attempts. Each has a descriptive `name`, `revolutions` captured per track, and host `retries` |
| `max_seconds` | Capture-budget ceiling, 30–1800 seconds per stage by default |
| `time_limit_scope` | `per_stage` (default, including omitted fields) or expert `whole_job` |
| `no_improvement_limit` | Stop after this many consecutive passes that fail to reduce unresolved sectors, 1–3 |

One to eight passes are allowed; each has 1–10 revolutions and 0–10 retries. Invalid settings are refused before reading. For everyday use, leave this alone and run `fv scan`.

A **profile map** is different: it records known disk formats, not retry effort. Your pilot's list says 009 is 720 KB and the other first twenty default to 1.44 MB. Once saved, it switches automatically during the scan.
