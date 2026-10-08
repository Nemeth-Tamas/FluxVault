# Crash-safe Windows host processes

No new scan flags or administrator setup. The current Windows executable supervises its external tools automatically. Normal `scan`, `scan --double` and saved-file processing use the same mechanism.

## Operator: stop and resume

- Prefer `QUIT` at a waiting prompt: finish active reads and drain saved-file work. Dual `PAUSE` blocks new feeding while current reads finish; it is not immediate cancellation.
- If the console closes or FluxVault crashes, its supervised GW/7-Zip/Office host processes and their descendants are terminated by Windows. Interrupted `.partial.scp`, `.partial.img` and metadata remain evidence, not successful completed acquisitions. Do not delete them to restart.
- Reopen the **same project**, use the **same scan command**, and follow its current custody prompt. Confirm the exact disk again; do not initialize/reset numbering or launch an independent raw reader. Dual resume still requires numbered `uN`/`gN` commands, not Enter-only mode.
- A complete hash-verified raw capture can resume decoding offline. An incomplete raw capture needs a new confirmed physical read and a new evidence attempt number. Previously saved acquisitions are not duplicated.
- A stopped host process is not electronic proof that the floppy motor has stopped or that an operator removed the correct disk. After an abrupt exit, wait for drive activity to settle and follow the reconfirmation prompt. Physical source access remains read-only.

If startup reports a controller/operation job restriction or suspended-host resume refusal, keep the error and retry from an ordinary PowerShell session. The program refuses unsupervised launches; it does not silently bypass the guard. No registry edits, elevation or firmware commands are needed.

## Engineering: two lifetime boundaries

Before CLI work or any library-managed host launch, the controller joins an unnamed Windows job with `KILL_ON_JOB_CLOSE`. Its single non-inheritable handle remains controller-owned until OS teardown. Children inherit job membership at creation, closing the startup race even if the controller dies before an operation's assignment completes. Initialization is thread-safe and failures are retained.

Each operation also owns a nested kill-on-close job. A host starts **suspended and hidden**, joins this job, and only then resumes. Stable Rust does not expose the initial thread handle; Toolhelp enumeration must identify exactly one thread of the suspended child, with the expected suspend count. Missing/ambiguous enumeration or assignment/resume failure kills the suspended host instead of running unmanaged code.

Normal host exit, timeout, wait failure or guard drop terminates remaining operation descendants. Before accepting a completed operation, Windows job accounting must confirm zero active processes within five seconds. Reader pipes are joined after operation cleanup, so a lingering child cannot retain stdout/stderr indefinitely after its leader exits. Operations have separate jobs: terminating one timed-out host does not terminate another operation or the controller. Existing command/recovery deadlines are unchanged; this is not a new bounded extraction deadline or immediate interactive cancellation feature.

Coverage: GW execution and version/health probes, audited extraction/7-Zip commands and isolated-profile LibreOffice conversions. It covers FluxVault-created hosts, not independently launched user applications. Existing read-only allowlists/device assertions are unchanged; no breakaway flags, resource/priority limits or administrator requirements are introduced. The controller-wide kill-on-exit implementation is Windows-specific; other platforms retain their existing runners without claiming this guarantee.

New `Logs/external-tools.jsonl` records identify `controller_supervision: "windows_controller_and_operation_jobs"`. Old records without that field remain readable and do not retroactively acquire a supervision claim. A forced controller exit may leave no final command audit record: partial artifacts/journals still retain interruption evidence. This field describes launch supervision, not file integrity or physical custody certification.

## Tests performed

`tests/process_supervision.rs` runs actual FluxVault/mock-GW executable processes without hardware:

- Forced single-GW and dual-controller termination during capture: retained leader/worker handles confirm exit; partial bytes stay unchanged; restart publishes exactly one clean image; an unrelated sibling process survives.
- Forced decode termination: partial image remains unchanged; restart reuses the complete SCP, with exactly one audited raw-read command across both invocations.
- Leader exits with a descendant holding output pipes: the CLI completes promptly and records supervision.
- Bounded operation timeout: entire child tree stops while the controller remains usable.

Unit tests cover concurrent initialization, missing-host refusal, failed resume of a suspended host, and old audit compatibility. The environment-dependent CLI fixture additionally runs numbered mock scanning with real 7-Zip/Office, checks extracted originals and DOCX/PDF, finalizes a CRC/hash-verified archive, and verifies that invalid-filesystem attention blocks finalization. Physical drive idle, every publication cutpoint, power loss and full production throughput remain distinct acceptance work.

Implementation references: [Windows job objects](https://learn.microsoft.com/en-us/windows/win32/procthread/job-objects), [nested assignment](https://learn.microsoft.com/en-us/windows/win32/api/jobapi2/nf-jobapi2-assignprocesstojobobject), [non-inheritable unnamed job handle](https://learn.microsoft.com/en-us/windows/win32/api/jobapi2/nf-jobapi2-createjobobjectw), [Rust child thread-handle API status](https://doc.rust-lang.org/stable/std/os/windows/process/trait.ChildExt.html).
