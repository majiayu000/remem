# Log Rotation Hardening Product Spec

Status: Current contract
Date: 2026-07-03

Tracking:
- Spec/tracking issue: #670
- Related runtime surfaces: host hooks, detached worker stderr, `remem doctor`

## Problem

`remem` writes operational diagnostics to `REMEM_DATA_DIR/remem.log` from many
short-lived processes: SessionStart/Stop hooks, MCP/API commands, install and
doctor commands, and detached background workers. The current log rotation
policy caps ordinary disk usage, but it is not reliable for this process model.

Current user-visible failure modes:

1. Concurrent hook or worker processes can race through the size check and
   file renames, because rotation has no cross-process lock.
2. Worker stderr is attached through `open_log_append()`, but that path opens
   the active log without running the same rotation policy as normal
   `write_log()` calls.
3. Retention is fixed at three rotated files, so users cannot tune log
   retention independently from active-file size.
4. Invalid log-related environment values silently fall back to defaults; the
   fallback is not visible in `doctor`.
5. Rotation failures are transient `eprintln!` messages. They disappear in
   hook contexts and are not surfaced through a durable diagnostic path.
6. A worker inherits an OS stderr file descriptor. If another process rotates
   the log later, that inherited descriptor may continue writing to the old
   file. Users need an honest limitation statement rather than an implied
   guarantee that every worker byte follows the latest active path.

These are not memory-quality features, but they protect memory-quality
debuggability: missing or truncated hook/worker diagnostics make capture and
extraction failures harder to diagnose.

## Decision

Preserve the existing lightweight file logger and default paths, but make the
prepare/rotate/open/write path concurrency-aware and diagnosable:

- Keep active log path `REMEM_DATA_DIR/remem.log`.
- Keep current defaults: 10 MiB active log and three rotated files
  (`remem.log.1` through `remem.log.3`).
- Add `REMEM_LOG_MAX_ROTATED_FILES` to configure retention.
- Add `REMEM_LOG_LOCK_TIMEOUT_MS` to bound waiting for the rotation lock.
- Serialize rotation and append preparation with a lock file under
  `REMEM_DATA_DIR` so hook, CLI, MCP, and worker-launch processes share the
  same critical section.
- Route both normal `write_log()` and worker `open_log_append()` through the
  same preparation policy before a handle is returned.
- If the lock cannot be acquired within the timeout, preserve the log line or
  stderr handle through append-only fallback and record a durable diagnostic
  that `doctor` can report.
- Report log health in `remem doctor`: active path, active bytes, total bytes
  across retained logs, configured retention, lock timeout, invalid env
  fallbacks, and the most recent rotation issue.

## Goals

- Prevent concurrent rotations from corrupting or losing normal hook/worker log
  writes.
- Ensure worker stderr setup triggers the same size/retention policy as
  ordinary logger writes.
- Let advanced users tune retained log count without changing the active log
  size threshold.
- Make invalid log configuration and recent rotation problems visible in
  `doctor` instead of disappearing into stderr.
- Preserve private-by-default local diagnostics by creating log and lock-related
  files with `0600` where the platform supports it.

## Non-Goals

- Do not replace the file logger with `tracing`, syslog, journald, or a remote
  logging backend.
- Do not write log data into SQLite or introduce a schema migration for this
  work.
- Do not change the active log path or rotate to date-based filenames.
- Do not guarantee that an already-running worker's inherited stderr descriptor
  is moved to the newly active log after a later rotation.
- Do not change capture, extraction, memory promotion, MCP semantics, or hook
  command shape.

## User-Visible Behavior

- `REMEM_LOG_MAX_BYTES` still controls active-log size and still defaults to
  `10485760`.
- `REMEM_LOG_MAX_ROTATED_FILES=5` keeps at most `remem.log.1` through
  `remem.log.5`; unset behavior keeps `.1` through `.3`.
- `REMEM_LOG_MAX_ROTATED_FILES=0` disables retained rotated files while
  preserving the active `remem.log` path.
- `REMEM_LOG_LOCK_TIMEOUT_MS` bounds how long a process waits for another
  process to finish rotating. On timeout, the current log write is still
  attempted through append-only fallback.
- `remem doctor` includes a log-health check. It reports `ok` for normal
  configuration, `warn` for invalid env fallbacks or recent rotation issues,
  and enough path/size/retention detail to diagnose disk-use problems.
- Documentation states that worker stderr is attached to a file descriptor at
  worker launch time. Later rotation by another process may leave that
  descriptor writing to the file it already opened until the worker exits.

## Behavior Invariants

1. Default compatibility: with no new env vars, the active file remains
   `remem.log`, the active-size threshold remains 10 MiB, and retention remains
   `.1`, `.2`, `.3`.
2. Retention: after rotation, no suffix above the configured retention count is
   left by the logger.
3. Concurrency: multiple independent `remem` processes crossing the threshold
   at the same time through `write_log()` do not lose the triggering log line,
   do not panic, and do not produce more retained files than configured.
4. Worker stderr preparation: `open_log_append()` creates the parent directory,
   applies rotation when needed, and returns an append handle to the active log
   or a documented fallback.
5. Permissions: newly created active logs, rotated logs, lock files, and
   rotation diagnostic files are created with `0600` on Unix, not merely
   chmodded after a wider initial mode.
6. Invalid configuration: invalid `REMEM_LOG_MAX_BYTES`,
   `REMEM_LOG_MAX_ROTATED_FILES`, or `REMEM_LOG_LOCK_TIMEOUT_MS` values fall
   back to documented defaults and are visible in `doctor`.
7. Failure visibility: lock timeouts and rotation/open/rename failures preserve
   the current write when possible and record a durable diagnostic that
   survives the failing process.
8. No recursive logging: logger-internal diagnostics do not recursively call
   the same logger path while it is holding or waiting for the log lock.

## Acceptance Criteria

- [ ] Subprocess concurrent-writer test proves all generated `write_log()`
      lines are present after rotation and retained suffixes do not exceed the
      configured count.
- [ ] `open_log_append()` test proves worker stderr setup rotates an oversized
      active log before returning a handle.
- [ ] Retention tests prove `REMEM_LOG_MAX_ROTATED_FILES=5` keeps `.1`
      through `.5` only, reduced retention removes stale higher suffixes, and
      `REMEM_LOG_MAX_ROTATED_FILES=0` leaves no retained suffixes.
- [ ] Invalid env-value tests prove defaults are used and doctor reports a
      warning.
- [ ] Lock-timeout and rotate-failure fallback tests prove the log line is
      preserved when possible and the durable diagnostic is visible to doctor.
- [ ] Unix permission tests prove new active, rotated, lock, and diagnostic
      files are created with `0600`.
- [ ] Documentation update covers the new env vars and the inherited worker
      stderr file-descriptor limitation.

## Risks

- Locking adds latency to hook processes. The timeout must be bounded, and
  fallback must preserve the log line rather than blocking capture.
- Lock files can be left behind after crashes. The design must rely on OS file
  locks, not lock-file existence, so stale files do not block future logging.
- Doctor must not read or print log contents. It reports paths, sizes,
  retention, and summarized issue metadata only.
