# Log Rotation Hardening Technical Spec

Status: Current contract
Date: 2026-07-03

Tracking:
- Spec/tracking issue: #670

## Existing Implementation Facts

Verified against `origin/main` (`0e8f80e`), 2026-07-03:

- `src/log/config.rs` defines `DEFAULT_LOG_MAX_BYTES = 10 * 1024 * 1024`,
  `LOG_ROTATION_KEEP = 3`, `log_path()`, `log_max_bytes()`, and
  `rotated_log_path()`.
- `src/log/write.rs::rotate_if_needed()` checks `metadata().len()`, then
  renames `remem.log.2 -> .3`, `.1 -> .2`, and `remem.log -> .1`. It has no
  cross-process lock and reports failures only with `eprintln!`.
- `src/log/write.rs::write_log()` creates the log directory, calls
  `rotate_if_needed(&path, log_max_bytes())`, opens `path` with
  `create(true).append(true)`, sets active-file permissions to `0600` on Unix,
  and writes the line.
- `src/log/write.rs::open_log_append()` creates the log directory and opens
  append mode for child stderr, but it does not rotate first and does not set
  file permissions after creation.
- `src/summarize/summary_job/worker_launch.rs::spawn_worker_once()` passes the
  `open_log_append()` file handle to worker stderr and sets
  `REMEM_STDERR_TO_LOG=1` so the worker does not mirror normal logger lines to
  stderr again.
- `src/doctor/database.rs::check_disk_space()` counts database bytes plus the
  active `remem.log` size only. It does not count rotated logs, retention
  policy, invalid log env values, or recent rotation/open failures.
- `docs/ARCHITECTURE.md` currently documents only `REMEM_LOG_MAX_BYTES`.
- `fs2` is already in use for file locks in worker launch code, so log locking
  can reuse the existing dependency.

## Proposed Design

### 1. Central log policy

Replace the single-purpose constants with a parsed policy object in
`src/log/config.rs`:

```text
LogPolicy {
  path: PathBuf,
  max_bytes: u64,
  max_rotated_files: usize,
  lock_timeout_ms: u64,
  invalid_env: Vec<InvalidLogEnv>,
}
```

Defaults:

- `REMEM_LOG_MAX_BYTES`: `10485760`
- `REMEM_LOG_MAX_ROTATED_FILES`: `3`
- `REMEM_LOG_LOCK_TIMEOUT_MS`: `250`

Parsing rules:

- `REMEM_LOG_MAX_BYTES` and `REMEM_LOG_LOCK_TIMEOUT_MS` accept positive
  integers only.
- `REMEM_LOG_MAX_ROTATED_FILES` accepts non-negative integers.
  `max_rotated_files = 0` disables retained rotated files while keeping the
  active log path.
- Invalid or overflowing values fall back to defaults and produce an
  `InvalidLogEnv` entry for doctor. The logger itself must not recursively log
  these parse failures.

Keep `with_log_dir()` for tests and preserve `log_path()` compatibility where
callers need only the path.

### 2. Locked prepare/open path

Add a shared prepare function in `src/log/write.rs`:

```text
prepare_log_file(policy, purpose) -> PreparedLog
```

Responsibilities:

1. Create the parent directory.
2. Open `remem.log.lock` with `create(true).read(true).write(true)` and, on
   Unix, `OpenOptionsExt::mode(0o600)` or an equivalent atomic create mode.
3. Ensure existing lock-file permissions are tightened to `0600` on Unix.
4. Try to acquire an exclusive OS lock until `lock_timeout_ms` expires.
5. Under the lock, rotate if `remem.log` is at or above `max_bytes`, then open
   the active log in append mode with `0600` creation mode.
6. For `write_log()`, write the current line while still holding the lock so a
   later process cannot rotate away the inode before the line reaches a
   retained file.
7. For `open_log_append()`, return the prepared append handle after rotation
   and handle creation; child stderr writes cannot hold the logger lock for the
   lifetime of the worker and remain covered by the documented inherited-fd
   limitation.

Use `fs2::FileExt::try_lock_exclusive()` in a short sleep/retry loop so hook
latency remains bounded. The lock is held around directory creation, rotation,
active handle creation, and the single `write_log()` line write. This prevents a
delayed writer from appending to an inode that another process has already
rotated out of the retained set.

### 3. Retention-aware rotation

Generalize `rotate_if_needed()` into a retention-aware helper:

```text
rotate_if_needed(path, max_bytes, max_rotated_files) -> Result<RotationOutcome>
```

Rules:

- Before any active-size short-circuit, remove every suffix above
  `max_rotated_files` that exists from prior higher-retention settings so
  reduced retention takes effect immediately even when `remem.log` is below
  `max_bytes`.
- If active size is below `max_bytes` after stale-suffix cleanup, do not rotate
  the active file.
- If `max_rotated_files == 0`, remove the active file instead of renaming it
  to `.1`; the next open recreates `remem.log`.
- For `N > 0`, remove `remem.log.N`, shift suffixes downward in reverse order,
  then rename active to `.1`.
- Ensure Unix `0600` permissions on every retained file after rotation where
  possible. New files must be created with `0600` atomically where Rust's
  platform APIs allow it; chmod is only a best-effort repair for existing
  files.
- Never panic on missing files. Missing rotated suffixes are normal.

The implementation may keep an internal compatibility wrapper for existing
unit tests, but new tests should exercise the configurable helper.

### 4. Append fallback and durable diagnostics

If the lock cannot be acquired within the timeout, `write_log()` must still try
to append the current line to `remem.log` without rotating. `open_log_append()`
must still try to return an append handle for worker stderr. If the lock is
acquired but remove/rename/open fails during rotation, the caller must also try
append-only fallback where possible before returning the error. All fallback
paths record an issue through a non-recursive sidecar diagnostic writer.

Use a small sidecar file under `REMEM_DATA_DIR`, for example
`remem.log.rotation-issue.json`, with `0600` permissions. It stores the most
recent issue only:

```json
{
  "kind": "lock_timeout|rotate_failed|open_failed|invalid_env",
  "message": "...",
  "path": "...",
  "at_epoch": 1780000000
}
```

This file must never contain log contents, secrets, hook payloads, or raw
stderr. It is a summarized health marker only. The writer must use direct file
I/O and must not call `crate::log::*`.

Sidecar writes must be contention-safe: write to a process-unique temporary
file in the same directory with `0600`, then atomically rename it over the
sidecar, or serialize an equivalent direct-write path. A truncated or
interleaved JSON sidecar is not acceptable because it removes the durable
diagnostic precisely when contention is highest.

Successful locked prepare/rotate/open operations clear the sidecar or update it
to a recovered state only when the sidecar issue timestamp is known to predate
that successful prepare attempt. A successful lock holder must not clear or
overwrite a newer timeout/rotation issue written by a concurrent fallback
process while it was holding the lock. Doctor only warns for unresolved issues
or issues inside a documented freshness window; an old transient timeout must
not make `remem doctor` warn forever after later healthy log preparations.

### 5. Doctor integration

Add a log-health check in `src/doctor/database.rs` or a new
`src/doctor/logging.rs` module and include it from `src/doctor/report.rs`.

The check reports:

- active log path;
- active log bytes;
- total bytes across active plus retained logs;
- configured `max_bytes`, `max_rotated_files`, and `lock_timeout_ms`;
- invalid env fallbacks;
- most recent unresolved or fresh rotation issue sidecar, if present.

Severity:

- `ok`: policy parses cleanly and no unresolved or fresh issue exists.
- `warn`: invalid env fallback, lock timeout, rotate/open failure, unreadable
  sidecar, or retained bytes exceed a documented warning threshold.
- `fail`: no fail state by default; logging should not block memory capture or
  doctor except for impossible internal errors. If future implementation finds
  a permission failure that prevents all logging, that can be promoted to
  `fail` with a test.

The existing `Disk usage` check should count rotated logs as part of log bytes,
or the new log-health check should make the distinction explicit so disk usage
does not under-report retained logs.

### 6. Documentation

Implementation PRs update:

- `docs/ARCHITECTURE.md` environment variable table with
  `REMEM_LOG_MAX_ROTATED_FILES` and `REMEM_LOG_LOCK_TIMEOUT_MS`.
- README troubleshooting or operations text only if the behavior becomes
  user-facing outside doctor.
- A note explaining that a running worker's inherited stderr descriptor may
  keep writing to the already-open file until that worker exits.

## Product-to-Test Mapping

| Product invariant | Implementation area | Verification |
| --- | --- | --- |
| P1 default compatibility | `src/log/config.rs`, `src/log/write.rs` | Existing log tests plus default-policy test |
| P2 retention | retention-aware rotation helper | `REMEM_LOG_MAX_ROTATED_FILES=5` and zero-retention tests |
| P3 concurrency | locked prepare/open/write path | subprocess writer test crossing threshold |
| P4 worker stderr preparation | `open_log_append()` | oversized-active-log test before returned handle writes |
| P5 permissions | log open, lock file, sidecar writer | Unix permission assertions behind `#[cfg(unix)]` |
| P6 invalid configuration | policy parser, doctor check | env parsing tests and doctor warning test |
| P7 failure visibility | sidecar diagnostic, doctor check | lock-timeout and rotate-failure fallback fixtures |
| P8 no recursive logging | sidecar writer | unit test or code structure proving it does not call `crate::log::*` |

## Data Flow

```text
write_log/open_log_append
        |
        v
parse log policy + invalid env collection
        |
        v
create parent dir + open remem.log.lock with 0600 create mode
        |
        +-- lock acquired -> rotate if needed -> open active append handle
        |                   -> write current write_log() line under lock
        |
        +-- lock timeout or rotate failure -> append-only fallback + atomic sidecar issue
        |
        v
write_log completed, or open_log_append returns worker stderr handle
```

`remem doctor` reads policy, file metadata, retained suffixes, and the sidecar
issue file. It does not read `remem.log` contents.

## Implementation Slices

Recommended implementation issue split:

1. Log policy + locked prepare/open path + configurable retention.
2. Durable diagnostic sidecar + doctor log-health reporting.
3. Documentation and worker-stderr limitation wording, if not included in the
   second slice.

The first slice is the minimum correctness fix. The second slice is required
before the epic can close because #670 explicitly requires diagnosable
fallbacks and invalid env visibility.

## Alternatives Considered

- **Use only SQLite for rotation issues.** Rejected for this slice: logging
  must work in hook/install contexts where the database may be unavailable,
  locked, or not migrated.
- **Block until lock acquisition.** Rejected: hook latency must remain bounded,
  and log rotation must not stall memory capture.
- **Switch to date-based logs.** Rejected: it changes user expectations and
  does not directly solve concurrent rename races.
- **Ignore worker stderr descriptor drift.** Rejected as silent degradation.
  The limitation remains, but it must be documented and doctor-visible enough
  to debug.

## Risks

- Security: log-health diagnostics must not include log contents, hook payloads,
  stderr text, secrets, or environment values beyond variable names and
  validity state.
- Compatibility: scripts may expect `.1` through `.3`; default behavior
  preserves that.
- Performance: lock acquisition must be short and bounded; slow paths use
  append-only fallback.
- Maintenance: logger internals must avoid recursively logging their own
  failures.

## Test Plan

- [ ] Unit tests: policy parser, rotated path generation, retention shifting
      including reduced-retention cleanup below and above the active-size
      threshold, invalid env collection, atomic sidecar serialization, and
      timestamp-safe recovery clearing.
- [ ] Integration tests: `open_log_append()` rotation, subprocess concurrent
      writers, lock-timeout fallback, rotate-failure fallback, doctor
      log-health warning.
- [ ] Regression tests: existing `src/log/tests.rs` coverage remains green.
- [ ] Manual verification: run `REMEM_LOG_MAX_BYTES=1 REMEM_LOG_MAX_ROTATED_FILES=5 cargo test log`
      or equivalent focused command in a temp data dir.

## Rollback Plan

The feature is additive and default-compatible. If locking causes unexpected
hook latency, set a low `REMEM_LOG_LOCK_TIMEOUT_MS` to force append-only
fallback while retaining diagnostics. A code rollback removes the locked
prepare path and new doctor check while keeping the old `remem.log` path and
existing rotated suffixes readable.
