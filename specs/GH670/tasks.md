# Task Plan

## Linked Issue

GH-670

## Spec Packet

- Product: `product.md`
- Tech: `tech.md`
- Authoritative docs contract:
  `docs/specs/log-rotation-hardening/PRODUCT.md` and
  `docs/specs/log-rotation-hardening/TECH.md`

## Implementation Tasks

- [ ] `SP670-T1` Owner: agent; Dependencies: spec approval; Done when: `src/log/config.rs` exposes a parsed log policy for max bytes, max rotated files including zero retention, lock timeout, invalid env diagnostics, and preserves default values; Verify: focused policy parser tests.
- [ ] `SP670-T2` Owner: agent; Dependencies: `SP670-T1`; Done when: `src/log/write.rs` serializes prepare/rotate/open/write for `write_log()` with `remem.log.lock`, configurable retention including reduced-retention cleanup, append-only timeout/rotate-failure fallback, and Unix `0600` creation modes; Verify: retention, permission, lock-timeout, and rotate-failure tests.
- [ ] `SP670-T3` Owner: agent; Dependencies: `SP670-T2`; Done when: `write_log()` and `open_log_append()` both use the shared prepare policy and worker stderr setup rotates oversized logs before returning a handle; Verify: focused `open_log_append()` regression test.
- [ ] `SP670-T4` Owner: agent; Dependencies: `SP670-T2`; Done when: logger-internal failures write a bounded non-recursive sidecar diagnostic atomically, clear or age out recovered transient issues, and never store log contents, secrets, hook payloads, or raw stderr; Verify: sidecar serialization, contention, recovery-clearing, and no-recursive-logging tests.
- [ ] `SP670-T5` Owner: agent; Dependencies: `SP670-T1` `SP670-T4`; Done when: `remem doctor` reports log path, active bytes, retained bytes, retention policy, lock timeout, invalid env fallbacks, and most recent rotation issue; Verify: doctor human/JSON tests.
- [ ] `SP670-T6` Owner: agent; Dependencies: `SP670-T1` `SP670-T2` `SP670-T3`; Done when: subprocess concurrent writers crossing the threshold preserve generated `write_log()` lines and never leave suffixes above configured retention; Verify: subprocess multi-writer regression test.
- [ ] `SP670-T7` Owner: agent; Dependencies: `SP670-T5`; Done when: documentation covers `REMEM_LOG_MAX_ROTATED_FILES`, `REMEM_LOG_LOCK_TIMEOUT_MS`, and the worker stderr descriptor limitation; Verify: docs diff plus `cargo fmt --check` / `cargo check`.
- [ ] `SP670-T8` Owner: agent; Dependencies: `SP670-T1` `SP670-T2` `SP670-T3` `SP670-T4` `SP670-T5` `SP670-T6` `SP670-T7`; Done when: local deterministic checks and CI pass, and #670 is updated with implementation evidence; Verify: commands below.

## Parallelization

Implementation should be mostly serial. `SP670-T1` through `SP670-T4` share
the logger internals and must not run as parallel writable lanes. After the
logger API is stable, `SP670-T5` doctor work and `SP670-T7` docs can proceed in
parallel if their writable files are disjoint.

## Verification

- `git diff --check`
- `python3 checks/check_workflow.py --repo .`
- `python3 checks/check_workflow.py --repo . --spec-dir specs/GH670`
- `cargo fmt --check`
- `cargo check`
- Focused tests for `log`, `doctor`, and worker stderr launch behavior
- `cargo test` before merge readiness

## Handoff Notes

Use `Refs #670` for partial implementation PRs. Do not close #670 until every
acceptance criterion in `product.md` is verified and documentation has landed.
