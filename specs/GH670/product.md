# Product Spec

## Linked Issue

GH-670

## Accepted Contract

The authoritative product contract is
`docs/specs/log-rotation-hardening/PRODUCT.md`.

This SpecRail packet hands the accepted #670 contract to implementation
planning. It does not replace the `docs/specs/` contract and does not approve
runtime implementation by itself.

## User Problem

`remem.log` is shared by short-lived hooks, MCP/API commands, install/doctor
commands, and detached workers. The current size-only rotation policy lacks a
cross-process lock, does not run for worker stderr setup, has fixed retention,
and hides invalid env/failure diagnostics outside durable doctor surfaces.

## Goals

- Preserve the existing active log path and default retention behavior.
- Serialize rotation and append preparation across processes.
- Apply the same rotation policy to `write_log()` and `open_log_append()`.
- Add configurable rotated-file retention and bounded lock waits.
- Surface invalid log env values and recent rotation/open failures in doctor.
- Document the inherited worker stderr file-descriptor limitation.

## Non-Goals

- Do not replace the lightweight file logger.
- Do not store log contents or rotation issues in SQLite.
- Do not change memory capture, context injection, MCP behavior, or worker job
  semantics.
- Do not guarantee that an already-running worker's inherited stderr descriptor
  follows later rotations.

## Behavior Invariants

1. Default behavior remains active `remem.log`, 10 MiB active threshold, and
   three rotated files.
2. Configured retention caps suffixes at the requested count, supports
   `REMEM_LOG_MAX_ROTATED_FILES=0`, and removes stale suffixes when retention is
   reduced.
3. Independent-process `write_log()` rotations preserve log lines and do not
   leave suffixes above the configured retention.
4. `open_log_append()` prepares and rotates the log before returning a worker
   stderr handle.
5. Newly created log, lock, and diagnostic files are created with `0600` on
   Unix.
6. Invalid `REMEM_LOG_*` values fall back to defaults and are visible in
   `doctor`.
7. Lock timeout and rotate/open failure fallback preserve append behavior where
   possible and record a durable diagnostic.
8. Logger-internal diagnostics avoid recursive calls into the logger.

## Acceptance Criteria

- [ ] Subprocess concurrent writer test covers multiple writers crossing the
      threshold.
- [ ] Worker stderr setup test proves `open_log_append()` rotates before open.
- [ ] Retention tests prove `REMEM_LOG_MAX_ROTATED_FILES=5` keeps `.1`
      through `.5` only, reduced retention removes stale higher suffixes, and
      `REMEM_LOG_MAX_ROTATED_FILES=0` keeps no suffixes.
- [ ] Invalid env test proves fallback plus doctor warning.
- [ ] Lock-timeout and rotate-failure tests prove line preservation plus
      doctor-visible issue.
- [ ] Unix permission test covers active, rotated, lock, and diagnostic files.
- [ ] Docs cover new env vars and the worker stderr descriptor limitation.

## Rollout Notes

Spec approval is still a human gate. Implementation should land in one or two
small PRs and use `Refs #670` until every acceptance criterion is complete.
