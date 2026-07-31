# GH933 Migration Rehearsal

Refs #933.

## Status

This is the mandatory evidence plan for the Phase A v2 cutover in
`MIGRATION-CUTOVER.md`. It is pending. A successful run is valid only for one
exact implementation HEAD and base SHA; any code, migration, dependency, or
schema change invalidates it.

The rehearsal never runs against the operator's live database. It uses a
byte-for-byte clone plus separately generated empty/current/legacy fixtures.
Production cutover remains human-authorized under `ROLLOUT.md`.

## Immutable Inputs

Record before execution:

- exact `git rev-parse HEAD` and merge base;
- Rust, Cargo, rusqlite, libsqlite3/SQLCipher, OS, and filesystem versions;
- source schema/user versions, database size, WAL mode, row counts, and
  SHA-256 of database, `-wal`, and `-shm` inputs;
- exact migration SQL SHA-256 and `sqlite_schema` fingerprint;
- local-copy root device/mount type and whether directory/file fsync is
  supported; and
- an encrypted, fsynced, read-only-test-opened pre-cutover backup and digest.

The run uses release mode where production does. Test clocks and fault points
are explicit; no randomized seed is left unrecorded.

## Required Commands

The implementation PR adds focused targets with these stable entrypoints:

```bash
cargo test gh933_sha256_frame_vectors -- --nocapture
cargo test gh933_retry_ledger_ddl -- --nocapture
cargo test gh933_writer_coverage -- --nocapture
cargo test gh933_idempotency_matrix -- --nocapture
cargo test gh933_local_copy_crash_matrix -- --nocapture
cargo test gh933_migration_rehearsal --release -- --ignored --nocapture
cargo test truth -- --nocapture
cargo test --test truth_public_api
cargo test
cargo clippy --all-targets -- -D warnings
```

The ignored release rehearsal writes canonical JSON to
`GH933_REHEARSAL_JSON_OUT`. The harness refuses a dirty worktree, missing output
path, non-release build, or HEAD mismatch.

## SHA-256/UDF Acceptance

The independent encoder, Rust helper, and SQLite UDF must return the same
lowercase digest for every golden frame. Tests assert the exact input frame
bytes, not only equality between two implementations.

Negative cases must reject: zero/odd arguments; non-TEXT, blank, non-ASCII, or
duplicate field names; unsupported SQLite value type; changed pair order; NULL
substituted for empty TEXT/BLOB; integer/real type substitution; and a
non-finite REAL entering from a DTO. Every owned production/migration/test
connection runs a known-vector self-test after keying and before schema access.

Apply the actual migration with the UDF unregistered and with an intentionally
wrong same-name UDF. The first fails `no such function`; the second fails the
known-vector preflight. Neither writes `user_version`, request rows, history,
or memory bytes. `cargo tree -e features -i rusqlite` must show `functions`.

## Executable DDL Matrix

Extract each installed SQL object from `sqlite_schema` and execute the exact
`MIGRATION-CUTOVER.md` block against isolated SQLite with `foreign_keys=ON`;
never use a relaxed copy. Normalize only insignificant whitespace and identifier
quoting, then require name/body equivalence for every object, explicitly
`memory_route_ledger_fingerprint_guard`,
`memory_lifecycle_ledger_fingerprint_guard`, `memory_insert_v1_ledgers`, and
`memory_route_tuple_update_guard`.

Positive cases cover all ten typed binding kinds and every allowed outcome:

| Binding | Required positive shape |
| --- | --- |
| `insert_origin` | inserted/backfilled memory plus exact route/lifecycle v1 |
| `route_transition` | changed memory plus one matching next route |
| `lifecycle_transition` | changed/acknowledged memory plus matching next lifecycle |
| `memory_outcome` | inserted/updated/reinforced/noop memory |
| `operation_outcome` | existing operation-log ID |
| `claim_outcome` | created/reused existing claim; disabled/failed without ID |
| `poisoning_ack` | acknowledged memory or explicit not-required/failed |
| `local_copy_outcome` | written path+digest or disabled/failed |
| `audit_outcome` | recorded scalar event ID or explicit not-required/failed |
| `response_aux` | exactly one canonical returned-result object |

For each row, recompute the binding fingerprint independently and verify the
ordered predecessor chain and final seal digest. Then run these negative cases,
each in a fresh transaction, and assert the named statement fails and leaves
every table/count/digest unchanged:

1. blank/space/uppercase-invalid writer/request identities and short, uppercase,
   nonhex, or overlong fingerprints;
2. invalid/nonarray/noncanonical/empty manifest, unknown kind, missing/extra
   object key, negative ordinal, duplicate pair, unsorted pair, or zero/multiple
   `response_aux` entries;
3. result absent from manifest, result out of manifest order, wrong predecessor,
   wrong fingerprint, unknown outcome, and every cross-kind typed-column leak;
4. each kind with one required ID/path/digest removed, a forbidden ID added,
   dangling FK, duplicate route/lifecycle binding, or noncanonical JSON;
5. seal with zero, missing, unexpected, or shape-invalid results; wrong terminal
   chain, response schema/JSON, request fingerprint, or result fingerprint;
6. inserted memory without an `insert_origin`, mismatched writer/request/ordinal,
   wrong memory ID, route/lifecycle ID, non-v1 row, predecessor, or source tuple;
7. transition result bound to another memory/request/ordinal/writer, or a Web
   lifecycle with missing/wrong-type API operation binding;
8. route/lifecycle INSERT with no compatible manifest slot, either ledger
   appended after seal, and seal with an otherwise valid but unbound ledger row;
9. all seven owner scopes accepted, but unknown scope, partial pair, empty,
   ASCII-whitespace-only, leading-whitespace, or trailing-whitespace key rejected;
10. result inserted after seal and duplicate request/result/seal insertion; and
11. UPDATE and DELETE against request, result, seal, route, and lifecycle rows,
   both before and after seal, plus mutation of a memory origin tuple; and
12. a changed route tuple with no staged next row, wrong head, OLD mismatch, or
   NEW mismatch; each aborts unchanged, while a matching terminal next row
   permits the update and null-safe same-value assignments need no route row.

For both fingerprint guards, enumerate every table column except row ID/digest
and prove the literal frame has exactly one ordered `old_*` and `new_*` field
(v1 OLD values are typed NULL) plus request fingerprints. Insert valid v1/v2
and mutate each OLD/NEW value while reusing the digest; each must abort. Recompute
the full frame and only the valid predecessor/version/status/time chain passes.
For `memory_insert_v1_ledgers`, run every insert family, missing/wrong
`insert_origin`, wrong UDF, invalid route/lifecycle value, and injected failure
between its two INSERT statements. A parent INSERT yields exactly memory+route
v1+lifecycle v1 or zero rows—never one ledger. Compare the three literal trigger
bodies independently with normalized `sqlite_schema.sql`.

Explicitly prove that an intent cannot commit without a seal and a seal cannot
commit without all manifested results. `PRAGMA foreign_key_check` is empty after
every positive case.

## Writer and Retry Matrix

Exercise the six current memory insert families and the three existing-row
transition writers named in `TECH.md`. For each, assert intent precedes mutation,
INSERT creates route/lifecycle v1 in the same transaction, every declared result
is typed, seal is last, and an injected error at each statement rolls everything
back. Same-value assignments append no transition.
One cleanup operation may bind distinct memories; repeating its operation/memory
pair is rejected.

Direct save acceptance is:

| Sequence | Expected durable effect |
| --- | --- |
| key A + payload P | one execution and sealed response |
| key A + exact P after response loss | same response; zero new rows/files/epochs |
| concurrent key A + exact P | one winner; loser returns winner |
| key A + payload Q | conflict before any mutation |
| key B + byte-identical P | second execution |
| lesson key C + P, then key D + P | reinforcement/operation/claim evidence advances twice |

Repeat with all Option-presence, CR/LF, outer-whitespace, list-order/duplicate,
local-copy, claim, acknowledgement, adapter-default, and reference-time
differences. Each behavior-bearing difference changes request fingerprint; key
or credential changes alone do not. Search database, journal, logs, traces,
errors, and response artifacts to prove no raw/normalized caller key appears.

Markdown retry remains stable across its own metadata rewrite, but different
stable source/no-source archive identities remain distinct.

## Migration Fixtures

Run foreground migration on:

- empty current schema;
- one row for each exact memory status (`active`, `stale`, `superseded`,
  `archived`, `deleted`, `rejected`) and scope/owner/topic
  NULL/empty/exact-value boundary;
- real anonymized legacy clone with pruned events and unsupported historical
  writers, requiring forward-only floors;
- surviving exhaustive evidence that legitimately reconstructs A→B→C;
- 100,000-memory scale fixture with FTS, graph, claims, operations, and WAL;
- malformed owner pair, status, chain, FK, FTS/schema object, or source version;
  and
- injected interruption before/after every migration stage and before COMMIT.

Positive fixtures preserve every unrelated column/index/trigger/FTS result,
create deterministic migration IDs, exact typed origins and seals, and match
terminal rows. Two independent migrations of identical bytes produce identical
logical rows/fingerprints excluding SQLite-assigned IDs documented as outputs.

Malformed fixtures fail closed without advancing `user_version`. Restart after
an interrupted transaction sees the exact old schema and succeeds once; a
second migration invocation is a no-op. Postflight requires
`integrity_check='ok'`, empty `foreign_key_check`, no unsealed intent, exact
manifest/result equality, contiguous ledgers, and current-row terminal equality.

## Local-Copy Crash Matrix

Fault injection kills the process, without unwinding, after every boundary:

| Boundary | No DB seal recovery | Matching DB seal recovery |
| --- | --- | --- |
| first deterministic temp create/write/fdatasync | prior target; owned temp found and removed | not reachable |
| first temp→canonical rename/directory fsync | prior target; canonical-or-temp scan converges | not reachable |
| `inspect_intent` fsync; owner-read lift/hash/restore/fsync | exact original mode/identity/bytes | not reachable |
| `stage_building` J fsync; U O_EXCL create; IU phase fsync before first byte; each chunk; fdatasync/D1 check | exact prior target; proved absent/empty/partial/full U removed; S never exists | not reachable |
| `stage_ready`; U→S no-replace; portable U/S link; Q/P fsync; `staged` | exact prior target; only proved U/full-D1 S removed | not reachable |
| `swap_intent`; B no-replace link, source recheck and P fsync; `backed_up` | exact target retained; remove only proved stage/B link, while a mismatched link source is preserved visible | not reachable |
| present-target S↔target exchange, identity postcheck, competitor-proof journal fsync and reverse compensation | exact exchange rolls back to D0; stable competitor is restored to target with J/B/S unsealed; drift preserves all | not reachable |
| absent-target S→target atomic no-replace and portable link/unlink | prior absence restored only without competitor; EEXIST preserves competitor/J/S as ambiguous | not reachable |
| target/parent fsync and `swapped` | prior target/absence restored | not reachable |
| each DB mutation/result before seal | prior target/absence restored | not reachable |
| seal INSERT before SQLite COMMIT | rollback; prior target restored | not reachable |
| SQLite COMMIT before journal `sealed` | keep exact after digest; cleanup | same |
| `sealed` journal write/fsync | keep exact after digest; cleanup | same |
| backup/stage unlink and parent fsync | keep exact after digest; finish cleanup | same |
| recovery-phase fsync and every recovery rename/unlink/fdatasync/parent fsync | restart same phase and converge | restart same phase and converge |
| journal unlink/directory fsync | keep exact after digest; no journal | same |

At every table boundary and both sides of each listed syscall, use separate OS
processes, not threads: the writer holds the retained L inode while a startup
scanner and a doctor/reconciler each attempt that exact lock. Cover pre-J,
`inspect_intent`, `reserved`, `stage_building`, every U create/write/fsync,
`stage_ready`, U→S, `staged`, `swap_intent`, B-link before/after `backed_up`,
present-target exchange/compensation or absent-target publish before/after `swapped`, every DB
result and seal step, COMMIT-before-`sealed`, `sealed`, each cleanup/J fsync, and
each recovery phase/action. Both contenders must return
`local_copy_writer_in_progress` (or bounded-wait at startup), and an event trace
must show no J/U/S/B/target open, classification or mutation and no R-scoped DB
read/write. Snapshot bytes, inode, digest and phase must remain unchanged.
Explicitly hold the writer with durable D1 at S and at target but no DB seal;
doctor must neither restore D0 nor delete D1/J/U/S/B.
The lock must still be busy after each cleanup unlink and after J unlink but
before Q fsync, and become acquirable only when the owner releases it afterward.

SIGKILL the writer at every boundary so the kernel releases L, then race scanner
and doctor as independent processes. Exactly one must acquire the same retained
lock inode and reconcile through terminal fsync/J cleanup; the other stays busy
or, after bounded wait, observes no work. Repeat with reversed process order.
Reject alternate lock paths/inodes, process-local mutexes, lock-file replacement
and PID/mtime stale-owner inference. Wrong lock/locks-dir path, identity, type,
uid, mode or link count must return `local_copy_lock_unsafe` before R inspection.

Run every boundary with prior target absent and present, an identical target,
multibyte bytes, and concurrent exact retries. Assert final target bytes/digest,
database counts, stored response, journal/artifact counts, and doctor status.
At every `stage_building` checkpoint use empty, one-byte, every chunk-prefix and
full U. Exact nonce/private-Q/type/uid/0600/nlink proof converges automatically;
wrong nonce/path/inode/uid/mode/type/link is preserved ambiguous. Precreate
wrong-proof U/S, race a second writer (blocked by L), mutate U after IU capture,
and forge wrong bytes at S: only a successful exact private-Q U proof may be
classified as partial; wrong-byte S is never normal crash state.

Race every source boundary with an independent process. Before B `linkat`,
rename-replace target so B may capture C rather than I0: the target+B postcheck
must reject before exchange or seal, preserve all names, and never classify C as
D0. After a valid B pin but before present-target exchange, replace target with
C: the exchange may move C to S, but remem must durably record `IC/MC/DC`,
reverse-exchange only exact target=D1/S=C, prove C restored at target and D1 at
S, and return `local_copy_publish_collision` with J/B/S and an unsealed DB.
SIGKILL before/after the competitor-proof fsync and reverse exchange; recovery
must finish the same compensation. Race the reverse itself: any changed source
or postcondition preserves every entry as ambiguous, with no unlink or seal.
If remem exchanges first and the competitor replaces afterward, the pre-seal
IU/D1 recheck preserves the competitor/evidence.

Separately race absent-target S→target. Linux `RENAME_NOREPLACE`, macOS
`RENAME_EXCL`, and portable `linkat` must return EEXIST when the competitor
wins, preserve it plus J/S, and leave the DB unsealed. Assert zero target→B
rename, zero plain replacement rename, zero competitor unlink, and no cleanup of
an identity-mismatched B/S in syscall traces.

Run a real filesystem probe for readable 0644 and unreadable owner-writable 0200
targets. Write known bytes before chmod; exercise journaled owner-read lift,
double hash, exact mode restoration, B hard-link pin, S↔target exchange and
reverse recovery. Assert dev/inode, uid/gid/mode/size/mtime, digest and bytes are
preserved; target/B then B/S are the exact temporary nlink=2 I0 pair, T/U remain
private-Q 0600, and B/S retain 0644/0200. Exercise D1-link checkpoints. Precreate
B and assert link fails before exchange; probe Linux `RENAME_EXCHANGE` and macOS
`RENAME_SWAP` before target mutation. Unsupported present-target exchange is a
visible no-mutation compatibility error.

Create distinct journal Q/locks directories at 0700 and a current-uid target
parent P at 0755 on Q's device, including a missing descendant securely created
via its parent dirfd. Prove L/J/T/U stay below Q, S/B/target stay below P, and
S inherits IU only after full-D1 atomic no-replace publication with exact
0600/current-uid/regular/nlink=1/entry-FD identity. Reject root/`..` escape,
symlink or
non-directory components, Q alias, wrong parent uid, missing owner rwx,
group/world-writable P, cross-device target, changed `(dev,ino)` or uid/gid/mode,
and missing directory-fsync, atomic no-replace, or required exchange support. Replace/rename the
parent after opening its dirfd at each revalidation boundary: operations must
remain bound to the proved P and publish nothing at the replacement path, or
fail visibly without mutation.

Keep all 23 historical double-crash vectors as named regressions with identical
final outcomes (old reserved+S is now `stage_ready`). Replace the unsafe
B-over-D1 recovery with durable `recover_before_file`: when target=D1 and
B/S are the proved I0 pair, exchange exact target/S, verify target=D0/S=D1,
unlink S then B with a P fsync after each; before exchange, remove proved S then
B. Inject after each action/fsync and retain absent-target D1 unlink plus sealed
S/B cleanup vectors; add U/D1-link, source-race and `compensate_intent` states.
Kill/restart at each, then again at every reachable later syscall to a fixed
point. Protocol states converge to D0/absence or D1; competitor states converge
to a restored stable competitor or remain byte-preserving ambiguity.

Also test canonical only, temp only (empty/partial/complete), both, the expected
retained locks subtree, unknown name, wrong T/S uid/mode/link count, B
identity/metadata/digest drift, inode alias and path escape. Mutate each accepted
physical cell to absent/D0/D1/wrong bytes;
only listed states converge, while every failed proof remains intact/ambiguous.

Tamper separately with journal JSON, nonce/IU/IC, request fingerprint,
target/backup/U/stage path, type, inode alias, metadata and each digest;
remove/invalidate the DB; create an
unexpected combination of target and backup. Every ambiguous case leaves all
user-visible files and journal untouched, returns
`local_copy_reconciliation_ambiguous`, logs at error with only opaque identity,
and makes doctor nonhealthy.

Inject cleanup permission/fsync failure after commit. The stored `written`
response remains the replay response, pending journal state is durable/visible,
exact retry reconciles before returning, and later doctor cleanup removes only
owned artifacts. There is no warning-only or swallowed cleanup failure.

## Evidence Artifact

The JSON artifact schema v1 contains:

```json
{
  "schema_version": 1,
  "head_sha": "<40 lowercase hex>",
  "base_sha": "<40 lowercase hex>",
  "migration_sql_sha256": "<64 lowercase hex>",
  "source_database_sha256": "<64 lowercase hex>",
  "backup_database_sha256": "<64 lowercase hex>",
  "tool_versions": {},
  "fixture_counts": {},
  "schema_fingerprint_before": "<64 lowercase hex>",
  "schema_fingerprint_after": "<64 lowercase hex>",
  "forward_only_route_count": 0,
  "forward_only_lifecycle_count": 0,
  "ddl_negative_cases": {},
  "ddl_trigger_equivalence": {},
  "idempotency_cases": {},
  "crash_boundaries": {},
  "stage_build_cases": {},
  "publish_collision_cases": {},
  "lock_contention_cases": {},
  "target_parent_cases": {},
  "filesystem_probes": {},
  "recovery_second_crash_cases": {},
  "integrity_check": "ok",
  "foreign_key_violations": [],
  "raw_key_leaks": 0,
  "passed": true
}
```

Every case maps to pass/fail plus expected and actual state fingerprints. The
artifact is canonical JSON, SHA-256 signed in CI provenance, attached to the PR,
and reviewed independently. Missing, skipped, flaky-retried-without-root-cause,
or head-mismatched cases make `"passed": false`.

## Stop Conditions

Stop cutover and return to implementation on any UDF mismatch, schema drift,
unsealed intent, permissive UPDATE/DELETE, missing/extra result, origin mismatch,
raw-key leak, silent cleanup error, ambiguous reconciliation mutation, FK or
integrity error, partial S, competitor overwrite, unexplained forward-only
count, nondeterministic rerun, or
failure outside documented rollback. Passing this rehearsal is necessary but
does not authorize merge or production migration.
