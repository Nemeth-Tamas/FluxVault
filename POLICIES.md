# Recovery policies, without homework

**You do not need to make a policy file.** A policy is simply the program's recovery budget: how many attempts it may make, how thorough they may be, and when it must stop. It prevents one stubborn floppy from consuming the whole afternoon or being reread indefinitely. FluxVault chooses whether to continue from the recorded results; you do not pick a pass at every disk.

## The normal command

```powershell
fv scan
```

In an existing scanned project, this reuses the saved drive, default format, per-disk format list, recovery policy and last-disk target. These are saved as values, so the original policy/map filenames do not need to be supplied again. Explicit expert flags override saved settings; incompatible changes are refused while a disk is pending.

In a fresh project, the current defaults are the Greaseweazle B station, 1.44 MB format, the normal built-in recovery budget and no end target. `fv scan --last-disk 20` adds a stopping point. Unknown mixed formats still require a known format list or explicit profile; automatic physical format discovery remains planned. The current customer's pilot already has its 009-DD override saved.

Every physical prompt asks you to check the label and open write-protect hole. Type `004` (or `4`) to confirm the displayed disk, or `QUIT` to finish. Blank input and the wrong disk number start no read. Older `READ 004` input still works for existing scripts, but is unnecessary.

## Which budget am I using?

| Budget | What it allows |
| --- | --- |
| Built-in normal | Up to four escalating passes, 600-second acquisition ceiling, stop after two non-improving passes |
| Today's saved pilot budget | Up to three passes, 180-second acquisition ceiling, stop after two non-improving passes |

Both stop early when the reported sector map is complete. Later passes target unresolved areas plus clean identity-control cylinders. More retries do not guarantee more files. Missing/conflicting bytes remain flagged, never guessed. Raw captures are retained.

Acquisition limits are not a total processing stopwatch: decoding, hashing, evidence verification, extraction and document conversion add their own work/timeouts. A completed job is verified/reused on restart, not automatically reread. If the first attempt produced **no raw file at all**, an explicitly confirmed retry can restart its bounded acquisition window after expiry, preserving the failed attempt and old start time. A job with any full/partial raw evidence does not receive that reset.

## Custom files are optional expert controls

Only if you want different limits, copy `policies/pilot-short.json`, edit your copy, and supply `--policy 'C:\path\to\my-policy.json'` when starting a new scan/job. Do not change a pending job's policy. Its saved values must match so a restart cannot silently change the plan.

| JSON field | Meaning |
| --- | --- |
| `passes` | Ordered attempts. Each has a descriptive `name`, `revolutions` captured per track, and host `retries` |
| `max_seconds` | Acquisition-budget ceiling, 30–1800 seconds |
| `no_improvement_limit` | Stop after this many consecutive passes that fail to reduce unresolved sectors, 1–3 |

One to eight passes are allowed; each has 1–10 revolutions and 0–10 retries. Invalid settings are refused before reading. For everyday use, leave this alone and run `fv scan`.

A **profile map** is different: it records known disk formats, not retry effort. Your pilot's list says 009 is 720 KB and the other first twenty default to 1.44 MB. Once saved, it switches automatically during the scan.
