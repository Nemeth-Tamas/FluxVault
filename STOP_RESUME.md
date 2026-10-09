# Stop now. Resume later.

Keep the same project. Never reset its numbering, delete partial evidence or initialize it again to resume.

## Everyday commands

```powershell
# From your existing project folder: start = scan, not a reset.
fv start --last-disk 136 --no-verify

# In the scan console: type STOP and press Enter to cancel active work.
# QUIT instead finishes active reads and drains saved-file work.

# From a SECOND PowerShell window, while the first is busy:
fv stop --project 'C:\full\path\to\your\project'
fv run status --project 'C:\full\path\to\your\project'

# Later, back in that SAME project:
fv scan --last-disk 136 --no-verify
```

Windows **Ctrl+C / Ctrl+Break** also requests cooperative cancellation. Restart the command to resume durable work; this does not suspend/reawaken a running process. Typed `STOP` is case-insensitive and works during scanning reads and waiting prompts without closing stdin. Standalone processing/conversion does not read typed commands: use Ctrl+C or the second-console command.

| Choice | Effect |
| --- | --- |
| Scan `QUIT` / `Q` | Stops feeding; finishes active reads and drains saved-file work. |
| Dual `PAUSE` / `p` | Blocks new feeding; existing reads/files finish. Persists across restart. `RESUME` / `r` enables confirmations but starts no read. |
| `STOP`, Windows Ctrl+C, or `fv stop` | Cancels active work cooperatively; preserves resumable state and waits for workers to return. |
| `fv run status` | Probes the project-local owner lock only. No board/drive query or proof of motor idle. |

**STOP requested is not STOPPED.** Keep disks seated until the original console prints its final **STOPPED** cue, then wait until physical drive activity has stopped before moving them. A stopped host cannot prove firmware/motor idle. USB cancellation occurs between reads: a currently blocked Windows driver read must return before its worker can finish. Do not close the window or kill processes to hurry this up.

## Preserved on resume

- Pending single-GW labels and dual station custody need confirmation again. Dual still requires exact `uN` / `gN`, not Enter-only mode.
- Completed, verified acquisitions are reused without rereading. Interrupted raw captures get new attempt slots; previous partial bytes/failure metadata stay intact. A host that buffers captures can legitimately leave an empty interrupted SCP.
- A complete hash-verified SCP interrupted during decoding resumes **offline** with no second physical read. Partial decodes never become completed acquisitions.
- Spent capture time remains charged to its stage; restarting does not reset its ten-minute allowance.
- Processing/packing tasks remain retryable. Hash-bound outputs remain reusable; interrupted Office output stays isolated, not promoted into delivery. `fv conversion issues` reports retained scratch; `fv conversion retry` retries saved issues.
- Cancelled packages retain `.partial.zip`; rerunning builds a new verified archive. Raw capture retirement still requires a verified packed archive and binding. Safely abandoned packing scratch can be reclaimed on resume after validating its ownership.

Saved-file commands need no inserted disk:

```powershell
fv processing resume
fv storage resume
fv conversion retry
```

`fv stop` controls single-GW, USB-only and dual scans; `process`; `processing resume`; `storage resume`; `conversion run` / `conversion retry`; and `package build`. `finalize` exposes its processing/package phases separately, not one atomic operation. Other expert commands are not registered for second-console stop; Windows Ctrl+C reaches their shared cooperative boundaries/supervised host runners. An inactive control lock does not certify independently launched applications have exited.

## Exit codes and scripts

| Code | Meaning |
| --- | --- |
| `0` | Completed within this command's scope, not complete customer recovery. Successful `fv stop` means request recorded, not target already stopped. |
| `3` | Attention/partial, unresolved recovery/conversion, or operator decision remains. |
| `2` | Invalid input/project, missing/refused tool, or operation/fatal failure. These share a code; inspect the error. |
| `130` | Operator cancellation. Resume the same project after the original console and drive stop. |

Progress/STOPPED go to stderr. `--json` keeps stdout undecorated. Error responses use `error.code = operation_cancelled` for cancellation, otherwise `operation_error`; a command finishing with a persisted summary may return that summary instead. Always check the exit code. Cancelled reads stay in telemetry but are not counted as faulty-disk acquisition failures.

## Guards and validation

Requests bind to the canonical project, OS owner lock and unique run generation. Old requests cannot cancel restarted work. Duplicate owners, inactive/wrong-project requests and malformed/oversized controls are refused or ignored. No caller-supplied PID is killed. Private `.fluxvault-*` controls stay outside customer packages. Source-read-only rules/device reservations/evidence checks remain unchanged; no admin, registry changes or new policy file.

Windows mock subprocess tests cover typed/separate-console single/dual cancellation, prompt stop without EOF, retained process-handle exit, partial preservation, exactly-once restart, stale requests, offline decode reuse and cancelled Office descendants with saved-issue retry. Unit tests cover simultaneous mocked USB/GW cancellation, packing retirement guards and partial-package refusal. Real Office/7-Zip whole-chain tests also pass.

Live **WinWord 1 / Mitsumi / GW B**, 2026-10-09: interrupted near track 13; retained failed metadata/empty buffered SCP and **18,095 ms** spent stage time. Same-label resume used four completed captures and produced one verified image in **157.8 s**; recovery reduced two missing sectors to one. Cursor advanced once to 002. Saved-only processing recovered **22 intact chain files** and exported attention audit/workbook, not complete-recovery certification. Evidence: `FluxVault-Test\GW-Stop-Resume-WinWord1-20261009-v1`.

See [crash supervision](PROCESS_SUPERVISION.md) for forced-exit boundaries. Physical idle, every power-loss/publication cutpoint and full-136 throughput remain separate acceptance. Windows console reference: [SetConsoleCtrlHandler](https://learn.microsoft.com/en-us/windows/console/setconsolectrlhandler).
