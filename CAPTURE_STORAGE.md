# Capture storage: safe packing and restart

## Beginner: leave it automatic

New scans pack completed captures in the background. Keep swapping when the scan says to; you do not need to manage compression or temporary folders.

Packing is lossless: the container preserves the exact original SCP bytes, including timing evidence. A sector image is not a substitute for that evidence. Raw retirement happens only after the container and its binding record have been published and independently verified.

After an interrupted session, run this **inside the existing project folder**:

```powershell
fv storage resume
```

This finishes durable packing tasks and checks abandoned temporary compression/decode copies. It uses saved files only, not the floppy drive or board. It does **not** finish extraction/conversion; use `fv processing resume` for that work, or resume the scan normally. [Stop/resume tutorial](TUTORIAL.md#beginner-stop-resume-and-results).

If storage reports an error, keep the project intact. Failed tasks and original evidence remain available for retry. Do not manually delete the last readable copy to make an error disappear.

## Advanced: explicit packing

```powershell
fv storage pack 7
fv storage pack 7 --retire-raw
fv storage benchmark 7
fv storage resume --json
```

The first command keeps raw. The second explicitly retires the raw working copy after successful verification; original bytes remain recoverable from the single SCP member of the container. `benchmark` measures compression without replacing evidence. These are separate optional commands, not a required sequence.

Artifacts under `Flux`:

| File | Purpose |
| --- | --- |
| `007_attempt_001.json` | Original acquisition identity; unchanged by packing |
| `007_attempt_001.scp.zip` | One exact original SCP, losslessly compressed |
| `007_attempt_001.scp.packed.json` | Original/container sizes, hashes and codec binding |
| `.fluxvault-storage-queue` | Durable pending tasks and OS-held worker ownership |
| `.fluxvault-storage-work-*` | Private capture-bound scratch copies, not acquisition evidence |

Decode, flux status, recovery resume, storage benchmarks and package export understand packed evidence. Decode materializes an isolated verified SCP and holds a shared per-capture lock until the consumer finishes. Packing/abandoned cleanup needs the corresponding exclusive lock; active readers are not interrupted.

### Abandoned-copy cleanup rules

New scratch directories contain a bounded ownership record: capture filename, original size/hash, purpose and schema. Normal completion/error cleans its own disposable scratch. After process termination, the next storage queue startup or explicit packing can reclaim abandoned scratch only after verifying a surviving raw or packed original.

Cleanup is non-recursive and accepts only the exact generated filenames for that purpose. Unknown entries, edited/invalid ownership, foreign capture identities, links/reparse points and legacy unbound `.fluxvault-pack-*`/`.fluxvault-unpack-*` scratch are preserved. If canonical evidence is corrupt or missing, abandoned materializations are retained too. A busy reader is skipped without treating it as a failure. No general project-folder sweep is performed.

Do not add your own files to these private directories. Added entries make the directory ineligible for automatic cleanup.

### What is tested

- Actual child-process termination at **11 packing checkpoints**: scratch creation, ZIP header/payload/sync/verification/publication, binding creation/sync/publication, pair verification and raw retirement. Durable tasks resume; original bytes stay recoverable, verified published files are reused and abandoned owned scratch is reclaimed.
- Three interrupted materialization checkpoints, including a complete verified temporary copy. Corrupting the canonical container makes cleanup refuse; restoring it permits offline cleanup without a pending packing task.
- Injected short writes followed by OS disk-full errors inside ZIP, binding and materialization writes. Unpublished partials are cleaned; raw or verified packed originals survive and retry succeeds. Tests do not fill the workstation disk.
- Eight cross-process rounds with two simultaneous readers, repeated packer refusal while either reader remains, identical final container/binding bytes and ownership release after a queue owner's process is killed.
- Active-reader skip, corrupt-container enqueue refusal, changed/foreign/unknown scratch preservation, malformed ZIP bindings and existing package/decode regressions.

These are process-crash and injected-I/O tests, not a power-loss/storage-controller durability certification. Packing now participates in [shared resource admission](BACKGROUND_PROCESSING.md#automatic-resource-admission): RAM/volume-space estimates, acquisition-first CPU admission and serialized background bulk I/O. Physical throughput and hard OS resource limiting are not implied.

## Saved-capture validation, 2026-10-07

Release commands on a new isolated copy of customer 007 packed **54,050,828 -> 12,343,928 bytes**, verified packed-only status, resumed storage successfully and materialized the capture for both benchmark levels. Both decompressions matched the original SHA-256; no private scratch remained afterward.

Original source hash before/after: `480a9dfc34db83acbf1f46ea13cbe0d83054eda54858824bbbac1d5b23318739`.

Retained validation project: `C:\Users\User\Desktop\FluxVault-Test\Storage-Resilience-007-20261007-v1`. The source pilot was not modified. This validates storage, not new recovery yield or another physical read.
