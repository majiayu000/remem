# GH933 Phase A v2 Rollout

Refs #933.

## Status and Authority

This is the normative rollout and rollback contract for the implementation in
`TECH.md` and cutover in `MIGRATION-CUTOVER.md`. It is pending. Nothing here
authorizes automatic merge, release, migration, or deletion. Each external step
requires the named human approval.

Phase A v2 ships only at 0.7.0 or another explicitly approved breaking boundary.
Published 0.6.x binaries cannot write the v2 schema.

## Roles

| Role | Required decision |
| --- | --- |
| implementation owner | exact-head tests, docs, version sync, and evidence |
| database reviewer | DDL, backfill, FK/trigger, and rollback approval |
| security reviewer | key handling, UDF, paths, journal, and fault cases |
| independent reviewer | contract-to-code and rehearsal artifact review |
| release owner | maintenance window, backup proof, canary, and release |
| operator | stop writers, run cutover, observe, and execute approved rollback |

No person may approve their own database/security review. Merge and production
cutover remain separate human gates.

## Phase 0 — Implementation Complete

Entry requires the five GH933 contract files reviewed and the implementation
branch based on the intended release base.

Exit requires:

- all implementation scope in `MIGRATION-CUTOVER.md` is wired, with no
  compatibility fallback or uninstrumented writer;
- focused tests, full Rust test/clippy, plugin version sync, version-bump check,
  PR preflight, and public API compile pass at exact HEAD;
- actual migration SQL and installed `sqlite_schema` match the reviewed DDL;
- security and database reviewers approve unresolved-risk lists; and
- the PR remains unmerged until an independent final review.

Failure returns to implementation; it does not waive a test or narrow the
contract.

## Phase 1 — Rehearsal and Shadow Observation

Run `MIGRATION-REHEARSAL.md` against a production-shaped anonymized clone and
archive the exact-head artifact. Separately run the new binary on disposable
clones of recent database snapshots, with all outbound writes disabled, to
measure migration time, disk amplification, forward-only counts, and doctor
results. Shadow mode never points at the live local-copy root.

Exit requires:

- rehearsal `"passed": true` with independent review;
- two deterministic reruns on identical input with matching logical
  fingerprints;
- backup restore drill completed and timed;
- cross-process lock-contention and target-parent evidence complete on every
  supported filesystem;
- migration duration and peak disk usage fit the maintenance budget with at
  least 2x measured free-space headroom;
- all forward-only counts explained from source evidence; and
- zero raw-key leaks, ambiguous journal mutations, UDF errors, integrity/FK
  violations, unsealed requests, or schema drift.

Any changed implementation HEAD invalidates this phase.

## Phase 2 — Maintenance-Window Canary

The release owner chooses one noncritical real installation and records explicit
operator consent. Before cutover:

1. announce downtime and stop every CLI, app, hook, worker, MCP, and scheduled
   writer;
2. prove no old process holds the database or any per-request local-copy lock;
3. reconcile journals to zero (retained L files are not pending journals) and
   require doctor healthy;
4. create, fsync, hash, and read-only test-open the database backup;
5. verify binary/checksum, exact approved HEAD, free space, and rollback owner;
6. capture preflight schema, counts, WAL state, and filesystem evidence; and
7. obtain release-owner go/no-go approval.

The operator runs the foreground migration once. No old writer starts afterward.
Before enabling v2 reads or writes, require postflight `integrity_check`,
`foreign_key_check`, schema fingerprint, origins/seals, terminal ledger equality,
ledger→manifest→typed-result equality, seven-owner/key CHECK probes, FTS query,
doctor, and one read-only truth smoke check.

Enable one controlled save at a time: no-local-copy new memory; update/noop;
intentional identical lesson with two keys; exact retry; conflict retry; then
local-copy absent/0644/0200 target. Verify stored responses, result bindings,
files, stable lock identity, target-parent proof and metrics after each. A
failure stops the canary immediately.

## Phase 3 — Canary Observation

Keep the canary on the new binary for at least 72 hours and through:

- one restart with automatic journal reconciliation, including the pre-rename
  `swap_intent` tuple, 0200-target/backup proof, initial temp cleanup, and a
  second crash inside each persisted recovery phase;
- writer/scanner/doctor contention at every local-copy phase, including durable
  D1 before seal, followed by single-owner reconciliation after writer death;
- normal hook, MCP/API, import, Markdown, candidate, governance, and cleanup
  activity that exercises every supported writer;
- event retention cleanup proving history remains intact; and
- an exact-key response-loss retry plus a different-key identical lesson save.

The release owner reviews metrics at 15 minutes, 1 hour, 24 hours, and 72 hours.
Exit requires zero stop-condition events and stable query correctness,
latency/disk within the approved envelope, no pending journal older than five
minutes, and no unexplained forward-only or replay conflict growth.

## Phase 4 — General Release

General release requires separate human merge and release approvals after the
canary report is attached. Synchronize Cargo, lockfile, Codex plugin, runtime
manifest, npm wrapper, server metadata, changelog, README, architecture, and
upgrade guide. Release notes must state:

- breaking 0.7 schema and required downtime;
- 0.6.x cannot open/write a migrated database;
- backup path/checksum verification and restore limitations;
- required save idempotency key and retry/intentional-duplicate semantics;
- doctor journal diagnostics and operator action; and
- how to disable v2 projection without deleting history.

Roll out in bounded cohorts with an explicit pause between cohorts. Do not
auto-update all installations or auto-migrate a database on first read-only
command. Each database migration requires backup proof and operator consent.

## Required Telemetry

Metrics and structured logs use opaque request IDs only:

| Signal | Page/stop threshold |
| --- | --- |
| UDF registration/self-test/hash mismatch | any |
| migration, integrity, FK, schema, or terminal-ledger error | any |
| unsealed intent, post-seal ledger append, or manifest/result/ledger/seal guard rejection | any |
| idempotency conflict | observe rate; stop on unexplained surge |
| exact replay that mutates rows/files/knowledge epoch | any |
| route/lifecycle gap, fork, terminal drift, or unexpected forward-only result | any |
| journal `cleanup_pending` | page after 5 minutes or repeated retry |
| per-request lock wrong inode/path/proof, replacement, or simultaneous owners | any |
| live-writer lock busy | expected briefly; any artifact/DB inspection or mutation by contender, or busy after owner death, stops |
| target-parent confinement/identity/uid/mode/device/fsync/no-replace proof failure | any; no mutation |
| backup source/identity/metadata/digest mismatch or ambiguous artifact proof | any; never mutate |
| recovery phase fails to converge after any repeated crash | any |
| raw idempotency key/credential detected in retained output | any |
| migration latency/disk above approved budget | stop current cohort |

Dashboards separate expected caller conflicts from internal ledger failures.
Logs include binary version, schema version, writer kind, opaque request ID,
journal phase, and error code without content or raw path secrets.

## Rollback Matrix

| Point | Allowed rollback | Forbidden action |
| --- | --- | --- |
| before migration transaction | restart 0.6.x after proving original DB intact | use an unverified backup |
| transaction failed before commit | keep writers stopped, verify rollback, retry fixed binary or restore tested backup | partial manual DDL repair |
| migration committed, zero non-migration seals | restore tested backup during downtime, then start 0.6.x | copy tables selectively |
| any v2 write sealed | keep 0.7 writer/schema; disable v2 projection/read surface | old binary, down migration, dropping ledgers, restoring stale backup |
| local-copy journal pending without seal | after acquiring R's retained L lock, reconcile to exact prior bytes | inspect/recover while L is busy; blind deletion |
| local-copy journal pending with seal | after acquiring R's retained L lock, retain exact sealed target and finish owned cleanup | restore prior target |
| ambiguous journal/database state | stop writes, preserve all bytes/journal, escalate to database+security reviewers | automatic repair |

Disabling projection changes only read routing/feature flags. It never removes
request, result, seal, route, lifecycle, origin, or journal evidence. A later
schema downgrade requires a separately specified export/import migration that
proves no write loss.

## Stop and Resume

Stop the current phase on any telemetry threshold, unexplained test/rehearsal
difference, missing approval, backup uncertainty, writer process left running,
lock/target-parent proof failure, or inability to reconcile local-copy state.
Preserve database, WAL, journal,
logs, exact binary, and evidence before investigation.

Resume only from the last fully satisfied phase after root cause, reviewed fix,
fresh exact-head tests, a full rehearsal rerun, and renewed human approvals.
Three failed fixes on one symptom require revisiting the design rather than
continuing rollout.

## Completion

Phase A v2 rollout is complete only after all cohorts pass the observation
window, release artifacts are version-synchronized, upgrade/rollback guidance
is public, no stop condition remains, and the release owner signs the final
report. GH933 remains open for any acceptance item in `PRODUCT.md`; rollout
completion does not imply automatic issue closure or merge.
