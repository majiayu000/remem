# Tech Spec

## Linked Issue

GH-670

## Product Spec

Product: `product.md`

Authoritative contract:
`docs/specs/log-rotation-hardening/PRODUCT.md` and
`docs/specs/log-rotation-hardening/TECH.md`.

## Codebase Context

| Area | Files | Current behavior | Why relevant |
| --- | --- | --- | --- |
| Log config | `src/log/config.rs` | `REMEM_LOG_MAX_BYTES` only; retention is constant `3`. | Need parsed policy for max bytes, retention, lock timeout, and invalid env diagnostics. |
| Log writes | `src/log/write.rs` | `write_log()` rotates then opens append; rotation has no cross-process lock. | Main correctness path for hook, CLI, MCP, and worker logger lines. |
| Worker stderr | `src/summarize/summary_job/worker_launch.rs` | `open_log_append()` returns a stderr file handle without rotation. | Worker stderr must start from the same prepared active log. |
| Doctor | `src/doctor/database.rs`, `src/doctor/report.rs` | Disk check counts DB + active log only. | Need log-health visibility and retained-log byte accounting. |
| Documentation | `docs/ARCHITECTURE.md`, maybe README | Only `REMEM_LOG_MAX_BYTES` is documented. | New env vars and stderr-fd limitation must be visible. |
| Tests | `src/log/tests.rs`, `src/doctor/tests.rs` | Existing tests cover max-bytes parsing, open append, and simple rotation. | Add concurrency, retention, fallback, permissions, and doctor coverage. |

## Proposed Design

- Add a central `LogPolicy` parser with defaults for active max bytes, rotated
  file count, lock timeout, invalid env diagnostics, and
  `REMEM_LOG_MAX_ROTATED_FILES=0` support.
- Add a locked prepare/open/write path using `remem.log.lock` and existing
  `fs2` file-lock support. `write_log()` writes its line while the lock is
  still held; `open_log_append()` only holds the lock through preparation and
  returns the worker stderr handle afterward.
- Generalize rotation to accept a configured retention count, including
  zero-retention behavior.
- Route both `write_log()` and `open_log_append()` through the same preparation
  path.
- On lock timeout or rotation/open failure, preserve append-only behavior where
  possible and write a non-recursive, atomic sidecar diagnostic such as
  `remem.log.rotation-issue.json`. Later healthy prepares clear or age out old
  transient issues so doctor does not warn forever.
- Add a doctor log-health check reporting policy, sizes, invalid env fallbacks,
  and the most recent summarized issue without reading log contents.

## Product-to-Test Mapping

| Product invariant | Implementation area | Verification |
| --- | --- | --- |
| P1 defaults | policy parser + write path | default-policy and existing log tests |
| P2 retention | rotation helper | configured `.1` through `.5`, reduced-retention cleanup, and zero-retention tests |
| P3 concurrency | lock + rotate/open/write | subprocess multi-writer threshold test |
| P4 worker stderr | `open_log_append()` | oversized-active-log append-handle test |
| P5 permissions | open/rotate/sidecar paths | Unix `0600` assertions |
| P6 invalid env | policy parser + doctor | invalid env + doctor warning tests |
| P7 fallback visibility | timeout/rotate fallback + atomic sidecar | lock-timeout and rotate-failure fixtures |
| P8 no recursion | sidecar writer | unit/code-structure test |

## Data Flow

```text
write_log/open_log_append
  -> parse LogPolicy
  -> create parent dir and private lock file
  -> acquire remem.log.lock within REMEM_LOG_LOCK_TIMEOUT_MS
  -> rotate if active >= REMEM_LOG_MAX_BYTES
  -> open active append handle with 0600 create mode
  -> write write_log() line under lock or return worker stderr handle
```

On timeout or rotation failure, the flow skips or abandons rotation, opens
append directly when possible, and updates the sidecar diagnostic for doctor.

## Risks

- Security: doctor must not print log contents or raw stderr.
- Compatibility: default suffixes and active path must remain unchanged.
- Performance: lock wait must be bounded for hook paths.
- Maintenance: logger failures must avoid recursive logging.

## Test Plan

- [ ] Unit tests: policy parser, retention shifting including reduced
      retention and zero retention, invalid env collection, sidecar atomic
      write/recovery clearing.
- [ ] Integration tests: subprocess concurrent writers, `open_log_append()`
      rotation, lock-timeout fallback, rotate-failure fallback, doctor
      log-health check.
- [ ] Existing checks: `cargo fmt --check`, `cargo check`, focused log/doctor
      tests, and `cargo test` before merge readiness.

## Rollback Plan

The runtime implementation is default-compatible. A rollback can remove the
locked prepare path and doctor check while keeping existing `remem.log` and
rotated suffix files readable.
