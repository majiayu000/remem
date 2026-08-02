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
cargo test gh933_operator_cutover_gate -- --nocapture
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

## Operator Authorization Matrix

Seed the pending breaking migration, then open it through ordinary read/write,
read-only, CLI, hook, worker, MCP, and API paths. Every path must return
`breaking_migration_requires_authorized_cutover` with identical database/WAL/
schema fingerprints. With writers stopped, plan must checkpoint WAL and close
handles before backup/main hashing; prove mode 0600, fsync, canonical bytes,
database identity/hash, empty WAL, schema/user/target, binary, backup digest,
nonce, expiry, and stable SHA-256. Apply must not checkpoint or change those bytes.
Apply rejects missing, uppercase, BLOB, stale, reused, altered, wrong-database,
wrong-binary, wrong-backup, or post-plan-writer-state input before live writes.
Only the exact digest creates `approved`. Fail every preflight boundary and prove retryability; retire an expired/mismatched approval only after exact pre-cutover proof, retain audit history, then approve one replacement. Retirement after `cutover_started` fails. Started restart completes target, resumes same approval from exact pre-cutover bytes, or fails manual-restore.

## Executable DDL Matrix

Extract each installed SQL object from `sqlite_schema` and execute the exact
`MIGRATION-CUTOVER.md` block against isolated SQLite with `foreign_keys=ON`;
never use a relaxed copy. Normalize only insignificant whitespace and identifier
quoting, then require name/body equivalence for every object, explicitly
`memory_route_ledger_fingerprint_guard`,
`memory_lifecycle_ledger_fingerprint_guard`, `memory_insert_v1_ledgers`, and
`memory_route_tuple_update_guard`, `memory_status_update_guard`, `memory_write_commit_guard`,
`memory_write_lock_anchors`, all six insert-once conflict guards, and every
append-only UPDATE/DELETE trigger.

Positive cases cover all ten typed binding kinds and every allowed outcome:

| Binding | Required positive shape |
| --- | --- |
| `insert_origin` | inserted/backfilled memory plus exact route/lifecycle v1 |
| `route_transition` | changed memory plus one matching next route |
| `lifecycle_transition` | changed/acknowledged memory plus matching next lifecycle and exact integer/API operation and audit bindings |
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

1. non-TEXT/NUL-tailed/blank/space/uppercase writer kinds; malformed internal/API request IDs (including preexisting referenced API parents) while mixed-case
   valid IDs remain accepted; and short, uppercase, nonhex, overlong, embedded-
   NUL-tailed TEXT, or length-valid BLOB fingerprints/nonces/digests;
2. invalid/nonarray/noncanonical/empty manifest, unknown kind, missing/extra
   object key, negative ordinal, duplicate pair, unsorted pair, or zero/multiple
   `response_aux` entries;
3. result absent from manifest, result out of manifest order, wrong predecessor,
   wrong fingerprint, unknown outcome, and every cross-kind typed-column leak;
4. each kind with one required ID/path/digest removed, a forbidden ID added,
   dangling FK, duplicate route/lifecycle binding, or noncanonical JSON;
5. seal with zero, missing, unexpected, or shape-invalid results; nonterminal or
   current-row-mismatching route/lifecycle state, wrong chain, response
   schema/JSON, request fingerprint, or result fingerprint;
6. inserted memory without an `insert_origin`, mismatched writer/request/ordinal,
   wrong memory ID, route/lifecycle ID, non-v1 row, predecessor, or source tuple;
7. transition result bound to another memory/request/ordinal/writer, route audit,
   lifecycle integer/API operation or audit; unknown lifecycle action, invalid
   action/status/source/version tuple, same-status `writer_transition`, Web missing/wrong-type API binding, or a
   `poisoning_ack` carrying audit provenance;
8. route/lifecycle INSERT with no compatible manifest slot, either ledger
   appended after seal, and seal with an otherwise valid but unbound ledger row;
9. all seven owner scopes accepted, but unknown scope, partial pair, empty,
   ASCII-whitespace-only, leading-whitespace, or trailing-whitespace key rejected;
10. result inserted after seal and duplicate request/result/seal insertion;
11. with `recursive_triggers=OFF`, `INSERT OR REPLACE` against every anchor,
   request/result/seal, route, and lifecycle PK/UNIQUE/partial-UNIQUE collision;
12. UPDATE and DELETE against those six append-only tables before/after seal,
   plus mutation of a memory origin tuple; and
13. an unchanged route successor, or changed route/status with no staged next row, sealed stage, wrong head,
   OLD mismatch, or NEW mismatch; each aborts unchanged, while a matching open
   terminal successor permits the update and same-value assignments need no row; and
14. stored non-INTEGER values (nonnumeric TEXT/BLOB/nonintegral REAL) for every integer-domain ID/version/ordinal/epoch/floor, including nullable fields, are rejected unchanged.
15. once lifecycle history references an API mutation row, every UPDATE of its action/resource/response/schema/audit/time or operation ID aborts; unreferenced construction may finish before binding.

For both fingerprint guards, enumerate every table column except row ID/digest
and prove the literal frame has exactly one ordered `old_*` and `new_*` field
(v1 OLD values are typed NULL) plus request fingerprints. Reject v2+ route or
lifecycle `insert|legacy_backfill` and lifecycle `baseline`; insert valid v1/v2,
mutate each OLD/NEW value with reused digest, and only a valid chain may pass.
For `memory_insert_v1_ledgers`, run every insert family, missing/wrong
`insert_origin`, wrong UDF, invalid route/lifecycle value, and injected failure
between its two INSERT statements. A parent INSERT yields exactly memory+route
v1+lifecycle v1 or zero rows—never one ledger. Compare the six literal trigger
bodies independently with normalized `sqlite_schema.sql`.

Exercise anchor DDL independently: only valid opaque R, INTEGER nonnegative dev
and epoch, positive INTEGER ino, and TEXT lowercase 128-bit nonce insert. Nonnumeric
TEXT/BLOB/REAL identity values, BLOB nonce, duplicate R/dev+ino, malformed nonce,
UPDATE/DELETE/OR REPLACE fail unchanged. A short `BEGIN IMMEDIATE` race with
different candidate inodes for one R commits at most one exact anchor. Preseed
another R with the candidate `(dev,ino)` and a zero-length crash-left L; the
combined R-or-IL lookup must reject before nonce write, leaving L bytes/mtime
and every anchor row unchanged.

Explicitly prove that an intent cannot commit without a seal and a seal cannot
commit without all manifested results. `PRAGMA foreign_key_check` is empty after
every positive case.

## Writer and Retry Matrix

Exercise the six current memory insert families, three existing-row route writers,
and every status writer named in `TECH.md`. For each, assert intent precedes mutation,
INSERT creates route/lifecycle v1 in the same transaction, every declared result
is typed, seal is last, and an injected error at each statement rolls everything
back. Same-value assignments append no transition.
For route/lifecycle transitions, attempt seal before the memory update and
require terminal mismatch with the transaction still recoverable; then perform
the matching update/result/seal successfully. A sealed row cannot authorize a
later bare memory update, and a request-owned nonterminal row cannot seal.
One cleanup operation may bind distinct memories; repeating its operation/memory
pair is rejected.

Direct save acceptance is:

| Sequence | Expected durable effect |
| --- | --- |
| key A + payload P | one execution and sealed response |
| key A + exact P after response loss | same response; zero new rows/files/epochs |
| concurrent key A + exact P | if the winner releases L within 5,000 ms, loser acquires L and returns that winner; after timeout loser reports busy and a later exact retry returns the winner |
| key A + payload Q | conflict before any mutation |
| key B + byte-identical P | second execution |
| lesson key C + P, then key D + P | reinforcement/operation/claim evidence advances twice |

Repeat with all Option-presence, CR/LF, outer-whitespace, list-order/duplicate,
local-copy, claim, acknowledgement, adapter-default, and reference-time
differences. Each behavior-bearing difference changes request fingerprint; key
or credential changes alone do not. Search database, journal, logs, traces,
errors, and response artifacts to prove no raw/normalized caller key appears.
For the local-copy row, hold L in one process and start a direct-save contender
for the same R. Before release, prove repeated nonblocking attempts perform no
artifact or R-scoped DB access; release within budget and require L revalidation,
normal reconciliation, then exact stored-response replay. Separately advance an
injected monotonic clock through 5,000 ms without release, require
`local_copy_writer_in_progress`, then release and prove the next exact retry
replays with zero new rows, files, or epochs. Scanner/doctor remain one-shot.
In the same process, acquire R once and attempt a second acquisition through a
different locks-directory FD. The second must return busy before opening L;
prove the first capability remains valid and a child process still observes the
kernel lock as busy. Then fork while held, release the parent, acquire fresh in
the child, close the inherited old capability object, and prove a third process
is still busy and the fresh capability remains valid. Only closing that fresh
capability permits reacquisition.

Markdown retry remains stable across its own metadata rewrite, but different
stable source/no-source archive identities remain distinct.

## Migration Fixtures

Run foreground migration on:

- empty current schema;
- one row for each exact memory status (`active`, `stale`, `superseded`,
  `archived`, `deleted`, `rejected`) and scope NULL/empty/`" global "`/exact-value
  boundary, proving rebuilt scope is exactly `project|global`, plus owner/topic;
- real anonymized legacy clone with pruned events and unsupported historical
  writers, requiring forward-only floors;
- surviving exhaustive evidence that legitimately reconstructs A→B→C;
- 100,000-memory scale fixture with WAL plus nonempty claims, edges, facts,
  embeddings, FTS and every other table whose FK references `memories`;
- production external triggers that select `memories`, plus every preexisting memory-owned UPDATE side-effect trigger including FTS/enrichment/version/archive/status; prove external triggers are dropped before table absence, while all owned UPDATE effects stay absent through A→B→C replay and are recreated byte-exact only after terminal C and dependent rows match stored bytes;
- malformed owner pair, status, chain, FK, FTS/schema object, or source version;
  and
- injected interruption before/after every migration stage and before COMMIT.

Positive fixtures preserve every dependent row and normalized table/FK/index/
trigger/FTS definition byte-for-byte, create deterministic migration IDs, exact
typed origins and seals, and match terminal rows. Two independent migrations of
identical bytes produce identical logical rows/fingerprints excluding
SQLite-assigned IDs documented as outputs.
Trace transaction state: WAL checkpoint, handle close, byte backup, fsync/hash
and backup test-open all complete with no migration write transaction; only then
does the live database reopen, verify FK OFF before one `BEGIN IMMEDIATE`, run
`foreign_key_check` precommit, commit, restore/verify FK ON, and repeat the check.

Malformed fixtures fail closed without advancing `user_version`. Restart after
an interrupted transaction sees the exact old schema and succeeds once; a
second migration invocation is a no-op. Postflight requires
`integrity_check='ok'` and empty `foreign_key_check` both before commit and after
FK re-enable, no unsealed intent, exact manifest/result equality, contiguous
ledgers, and current-row terminal equality.

## Local-Copy Crash Matrix

Fault injection kills the process, without unwinding, after every boundary:

| Boundary | No DB seal recovery | Matching DB seal recovery |
| --- | --- | --- |
| every direct save (including local-copy disabled): virgin existence preflight; L create/candidate lock; serialized absence recheck; nonce write/fdatasync; locks-dir fsync; plain K insert/select/commit and fd/path/K/nonce recheck | only a fully virgin R may initialize; any old DB/artifact state fails before nonce/K mutation; retry otherwise exact-matches K | not reachable |
| first deterministic temp create/write/fdatasync | prior target; owned temp found and removed | not reachable |
| first temp→canonical rename/directory fsync | prior target; canonical-or-temp scan converges | not reachable |
| `inspect_intent` fsync; owner-read lift/hash/restore/fsync | exact original mode/identity/bytes | not reachable |
| `stage_building` J fsync; U O_EXCL create; IU phase fsync before first byte; each chunk; fdatasync/D1 check | exact prior target; proved absent/empty/partial/full U removed; S never exists | not reachable |
| `stage_ready`; U→S no-replace; portable U/S link; Q/P fsync; `staged` | exact prior target; only proved U/full-D1 S removed | not reachable |
| `new_pin_intent`; S→N no-replace; S/N proof and P fsync; `new_pinned` | exact prior target; prepublication cleanup may remove only proved N then S | not reachable |
| `swap_intent`; B no-replace link, source recheck and P fsync; `backed_up` | exact target retained; remove proved unexposed N/stage directly, but remove B only after target=B eligibility and durable `cleanup_intent`; a mismatched target source stays visible | not reachable |
| durable `exchange_intent`; present-target S↔target exchange; identity postcheck; `restore_intent`; C no-replace link/fsync; `restore_ready`; target→H no-replace; C/H→target no-replace; postcheck/P-fsync; `quarantine_intent`; N→G no-replace and P plus Q/quarantine fsync; `quarantined`; cleanup boundary | exact normal exchange restores D0 while G permanently retains D1, then persists `cleanup_intent` before H/S/B/C removal; pre-boundary drift leaves newest bytes at target or durably under H/N/G with visible collision; no reverse exchange or unbounded last-pin unlink | not reachable |
| absent-target S→target atomic no-replace or portable link/unlink (including `{target,S,N}` nlink=3), followed on no-seal by N→G and observed target=G→H classify/unlink-or-restore | uncontested prior absence plus retained G is terminal only when target/H are absent; target≠G stays, H=G may unlink, H≠G restores no-replace, and EEXIST preserves target/H/G collision | not reachable |
| target/parent fsync and `swapped` | prior target/absence restored | not reachable |
| each DB mutation/result before seal | prior target/absence restored | not reachable |
| seal INSERT before SQLite COMMIT | rollback; prior target restored | not reachable |
| SQLite COMMIT before journal `sealed` | matching seal is authoritative while J remains `swapped`; persist `sealed`, then reconcile | same |
| `sealed`; prior-file `predecessor_quarantine_intent`; B→O no-replace; P and Q/quarantine fsync; `predecessor_quarantined` | not reachable without seal | target/N keep exact D1; exactly B or O plus S keeps the same structural I0* across every crash while mode/content may drift; O becomes permanent before S cleanup and receives all late old-D0-FD writes. Prior absence creates no O |
| `cleanup_intent` source-J/Tc transition; independent J→mode-qualified V read lift; snapshot/J fsync; post-persist revalidation; every ordered source→Xc no-replace rename, Q/source-parent fsync, postcapture proof, Xc unlink/Q-fsync, mismatch restore, and empty-prefix revalidation | exercise exact `[B]`, `[H,S,B,C]`, and `[H]` sources | exercise exact `[S,N]` and `[N]` sources. Before the boundary, injected target replacement/write/chmod is retained or collides; recovery restores V then any Xc before prefix derivation and returns typed `local_copy_cleanup_concurrency_violation` with J/pins and nonhealthy doctor. After successful revalidation target/nonpermanent pins stay quiescent; G/O-backed mode/content drift remains allowed and every crash resumes the exact prefix |
| recovery-phase fsync and every C link, present/absent target→H rename, C/H→target link/rename, N→G or B→O rename, owned unlink/fdatasync and P/Q-quarantine fsync | restart the same phase/prefix and converge without pathname target unlink, overwrite, or pre-boundary loss of the last D1 name | restart same phase and converge with old D0 permanently under O |
| journal unlink/directory fsync | keep exact after digest; no journal | same |

At every table boundary and both sides of each listed syscall, use separate OS
processes, not threads: the writer passes one mandatory R/PID-bound held-lock
capability and holds the anchor-verified retained L inode while a startup
scanner and a doctor/reconciler each attempt that exact lock. Cover pre-J,
`inspect_intent`, `reserved`, `stage_building`, every U create/write/fsync,
`stage_ready`, U→S, `staged`, `swap_intent`, B-link before/after `backed_up`,
`new_pin_intent`, S→N, `new_pinned`, `exchange_intent` fsync,
present-target exchange/restore pins or absent-target
publish before/after `swapped`, every DB
result and seal step, COMMIT-before-`sealed`, `sealed`,
`predecessor_quarantine_intent`, B→O, `predecessor_quarantined`, `cleanup_intent`,
active V/Tc/Xc, each capture/restore/cleanup/J fsync, and
each recovery phase/action. Both scanner/doctor contenders must return
`local_copy_writer_in_progress`, and an event trace
must show no J/U/G/O/S/B/N/C/H/target open, classification or mutation and no R-scoped DB
read/write except the immutable K lookup needed after candidate acquisition.
Snapshot bytes, inode, digest and phase must remain unchanged.
Close the caller's original locks-directory FD after acquisition and prove the
capability's retained duplicate remains valid. A forked child must reject the
inherited capability, while a separately acquired contender remains busy.
Replace canonical Q, replace L, and substitute a decoy `Q/locks` at every
callback/rename/unlink/J boundary: return typed lock-unsafe before mutation and
preserve both the old canonical J and any replacement-path victim. Close the
capability mid-callback and prove the lock error propagates rather than becoming
boolean revalidation failure. Invalidate L during fresh inspection-J validation,
after existing-J replay restoration, and on both empty-return paths; also replace
canonical Q immediately after T→J. Inspection must not return success after any
such boundary. L's inode/name is never removed; SIGKILL releases
the kernel lock and a later acquisition reuses that same retained inode.
Forge the initial inspection entry's target identity, mode, and path; substitute
a decoy `target_parent` FD containing the same basename; and independently
precreate target-parent B/S and the real `U=Q/.R.<stage_nonce>.stage-build`.
Also call source-J preparation with a noncanonical journal basename. Each call
must fail typed before J/T creation or replacement, mode lift, or user-file
mutation, preserving canonical and alternate names plus target/pin bytes.
Repeat final-return invalidation for every successful source/cleanup journal
load, prepare and transition; cleanup-intent persistence/revalidation/ordered
cleanup; read-lift begin, finish, restore and recovery; snapshot proof; and
capture/restart path, including value, `None` and `False`
returns. Invalidate L or replace canonical Q/`Q/locks` after the last semantic
or path read but before return. The boundary must return typed lock-unsafe,
perform no later mutation, and preserve the last durable state rather than
report stale success.
After source or cleanup J exists, rename the proved target parent while retaining
its FD and recreate its canonical pathname as an empty directory or decoy.
Source snapshot, cleanup-J load, and terminal J unlink must each reject typed;
the old target/pins, canonical decoy, and J remain byte-identical.
Repeat the retained-P/canonical-P split during initial inspection immediately
after the missing/existing J read and before owner-read restoration, temp create,
T→J, replay/empty return; and at V begin, finish, restore, and recovery before
J→V, fchmod/fsync, V unlink, and final return. Each boundary re-resolves canonical
P and returns `local_copy_reconciliation_ambiguous` on mismatch, never operates
through the decoy, and preserves old P, decoy, J/V, and pins. If owner-read was
already added, only durable restoration through the retained exact FD/alias may
precede the error; J/V stays armed until canonical recovery is safe.
Enumerate every closed J phase against no seal, its matching seal, and a
mismatched seal. The only COMMIT-before-J-`sealed` positive case is
`swapped` plus matching seal; `sealed`, predecessor-quarantine and recover-after
require matching seal, recover-before/restore/rollback-quarantine/collision
require none, and `cleanup_intent` admits only its five exact source tuples.
Reject all other
cross-products, any unknown phase, and any J `goal` or goal-like field.
For each cleanup source, positively prove exact source-J format/request
fingerprints, epoch, phase, `before_kind`, publication/seal state,
`semantic_d0_digest`/`semantic_d1_digest`, canonical component paths, and
`source_namespace` dev/ino/uid/gid/type/nlink facts for the five source tuples.
Alias groups must be structurally equal with exact nlink; the cleanup document,
not source J, owns the ordered list and freshly observed mutable snapshot.
Permit coherent pre-boundary mode/size/mtime/digest drift after source-J fsync,
then prove cleanup freezes those current values. Forge identity/type/owner/nlink,
add an unrecorded hard link in both source and cleanup phases, add/remove an
entry, or corrupt a semantic field; reject with zero V creation or chmod.
Call source-J preparation with a structurally valid but non-live
`source_namespace`, and call source-to-cleanup transition with a mutable
snapshot whose structural projection differs from persisted source J or whose
mode/size/mtime/digest differs from a fresh live proof. Reject before Tc
creation, chmod, or J replacement. Conversely, after permitted coherent
mutable drift, cleanup conversion must freeze the live mutable facts while its
structural namespace still exact-matches source J. Make the first complete
ordered scan invoke a callback that changes an already-proved present entry,
and separately creates an entry that was already proved absent. Before any
mutation or success, a second complete callback-free scan must revisit every
present and absent predicate, disagree with the first/expected state, and
return typed drift while preserving J and all names. Guard every existence
predicate individually before and after its filesystem call. Crash the
source-J→Tc→J transition at O_EXCL create, zero bytes, arbitrary malformed and
every JSON byte prefix, fdatasync/rename/Q-fsync. Enumerate every same-R Tc
candidate across current, stale, malformed and multiple nonce/name forms:
canonical source J may discard/Q-fsync provisional bytes only at the sole exact
current-nonce basename, and only its complete exact document may advance.
Every other form is preserved ambiguous; canonical cleanup J rejects all same-R
Tc candidates. Independently crash creation of
`V=.R.<stage_nonce>.read-lift.<group>.<mode:04o>` and require exact
same-inode/bytes J/V nlink=2. Scan all same-R nonce/name candidates; stale,
malformed group/mode, an extra-dot same-R lexical prefix, or multiple candidates remain intact and fail closed,
while valid current-nonce V+Tc restores the encoded mode before touching Tc.
Prove a distinct valid neighboring request prefix stays isolated.
For each cleanup-list member create
`Xc=.R.<stage_nonce>.cleanup-capture.<H|S|B|C|N>` and kill after native
source→Xc no-replace rename, Q fsync, source-parent fsync, retained-FD/Xc
postproof, Xc unlink, and final Q fsync. Restart with cleanup J+Xc must first
restore Xc→source no-replace, fsync source parent then Q, and only then derive
the removed prefix; a destination EEXIST retains Xc and returns concurrency.
Source J+Xc, Tc+Xc, V+Xc, Xc during journal transition/removal, malformed or
multiple Xc, stale nonce, wrong ordered member, symlink, wrong uid/type/device,
and unsupported native no-replace remain untouched and ambiguous. Run the
V, Tc and Xc scanners under optimized execution and statically reject both AST
`assert` nodes and hand-raised `AssertionError` as production safety gates. Call capture/restart directly with J
absent/replaced/noncanonical, any Tc or V, or an Xc not naming the first
still-present ordered member; each must preserve every entry and return typed
ambiguity before mutation. Rewrite canonical bytes on the same J inode with an
invalid field or path binding and require direct capture/restart to reject it
after checking the complete intent, trusted-root and retained-handle contract.
Replace the user target so B becomes the old inode's sole name, then pass direct
capture a caller-forged nlink=1 proof; normal and optimized execution must derive
the still-named alias count internally, reject, and preserve B/J with no Xc.
With a valid J→V marker, replace B by a different inode and pass forged
snapshot/removed/exempt inputs selecting it; recovery must derive those inputs
from persistent J, leave both inode modes and bytes unchanged, and retain J/V.
Also pass a forged namespace into direct snapshot proof, a mismatched mode into
V begin, and call V finish while the canonical inode is still read-lifted; each
must reject without changing the inode or disarming V in ordinary or optimized mode.
Repeat the forged namespace, snapshot, source-tuple, removed-prefix,
exempt-identity and recovery-input cases with hostile `dict`/mapping, list and set subclasses whose
item access, iteration or equality lies, raises, or changes values. Non-exact
containers may not authorize proof. Recovery must derive deep built-in
snapshot, removed-prefix and exempt values from canonical persisted J plus the
current namespace, reject mismatches with the exact request in the typed error,
and preserve J/V/Xc/pins.
Repeat with a plain outer cleanup intent whose nested journal, trusted-root, or
directory proof is a hostile dict subclass; direct capture, public revalidation,
and ordered cleanup must reject before creating Xc or unlinking a name and must
preserve the last B alias plus J. Pass hostile handle/document subclasses through
every public journal boundary and a cyclic plain-builtin snapshot through
transition; also pass a 2,000-level acyclic plain-dict proof through direct path
proof, capture, revalidation, and ordered cleanup. No accessor may run, bounded
stabilization must reject before recursion exhaustion, and every failure is typed. Pass a non-exact
path contract to inspection and require zero temp/J creation. With L absent,
hostile PathLike conversion and every snapshot/read-lift hostile mapping accessor
must remain uninvoked because lock proof precedes external conversion or access.
For every public journal, snapshot and marker scalar, pass `str`/`int`
subclasses and `bool` in the integer slots; no subclass hook may run and the
typed boundary must reject before mutation. Exact built-in request, 32-lowercase-
hex nonce, canonical journal basename/group and in-range mode/ordinal are the
only accepted scalar forms. Exercise `load_cleanup_journal(path_contract=None)`
as the sole optional-contract compatibility route: swap canonical Q or
`Q/locks` immediately before and immediately after each bootstrap filesystem
operation and require typed lock-unsafe with both old and replacement trees
untouched. An explicitly supplied invalid path contract remains a typed error.
Inject both `set_inheritable` acquisition failures and consuming close-result
diagnostics for held/fork-inherited FDs, acquisition rollback, canonical-directory
traversal, temporary-root/reopened/executor-owned directory finalizers, and the
write-only capture writer, plus a mid-proof `fstat` error and path/Q replacement
inside caught no-replace failures. Each close fixture delegates exactly one real
close and only then raises its diagnostic; the owner is invalidated before that
call, and the returned numeric FD is never probed or retried. Attempt every
distinct sibling reader/writer/directory close once, preserve the first error,
retain later errors as diagnostics, release/reset matching capability and registry
state after ownership is consumed, leave any recycled decoy FD untouched, and
prove the request can immediately reacquire.
Raise one exact callback exception while the capture writer's consuming close
also fails: snapshot/revalidation must preserve the callback object's identity,
with close only diagnostic. Then inject a restoration/finish lock-safety error
after the callback and require that safety error to win while retaining the
callback diagnostically. Across the public cleanup boundary, common callback
errors must become typed reconciliation ambiguity with the original object as
cause, never `False` or cleanup-concurrency; ordinary proof drift still returns
`False`, P replacement remains ambiguity, and Q replacement remains lock-unsafe.
Call J unlink directly while any ordered name remains; require typed ambiguity,
the complete terminal namespace, and J to remain in ordinary and optimized runs.
At `before_journal_unlink`, inject a stale-nonce Tc and require J/Tc to remain;
wrong-mode/nonregular/unstable J and snapshot proof failures must expose typed
ambiguity, never raw OS, attribute, or assertion errors; canonical non-object
JSON such as `[]` must also remain byte-identical and return typed ambiguity in
normal and optimized execution. A safe pending Xc is regular, current-uid, on Q's device and
nlink≥1; it is intentionally not snapshot-inode-bound so a proof→rename
replacement is restored rather than orphaned or deleted.
Explicitly hold the writer with durable D1 at S/N and at target/N but no DB seal;
doctor must neither restore D0 nor delete D1/J/Tc/V/U/G/O/S/B/N/C/H.
The lock must still be busy after each cleanup unlink and after J unlink but
before Q fsync, and become acquirable only when the owner releases it afterward.

SIGKILL the writer at every boundary so the kernel releases L, then race scanner
and doctor as independent processes. Exactly one must acquire the same retained
lock inode, exact-match K and reconcile through terminal fsync/J cleanup; the simultaneous
other returns busy, while a separately launched later contender observes no
work. Repeat with reversed process order.
First prove the raw primitive's replacement hazard: A locks old L, rename-replace
the path, and B locks the new inode concurrently. Race both anchor transactions
with K absent in each DB ordering; at most one exact fd/path/K/nonce tuple becomes
a verified owner, while the other returns `local_copy_lock_unsafe` before J or
request-intent access. Repeat with preexisting K, replacement before/after every
recheck, copied/wrong/empty nonce, inode alias/reuse, duplicate `(dev,ino)`, and
scanner/doctor missing K. Reject alternate paths, PID/mtime inference, and a
process-local mutex used instead of the kernel lock. Require the narrow atomic
same-process reservation before opening L, then the OS lock and capability proof;
wrong lock/locks-dir identity/type/uid/mode/link count/content
or anchor mutation/mismatch must fail after at most K lookup.
At every local-copy API boundary accept internal R only when it exactly matches
`[A-Za-z0-9][A-Za-z0-9_-]{0,127}`. Cover one and 128 bytes plus mixed case,
underscore and hyphen; reject empty, 129 bytes, leading punctuation, dot, slash,
space, non-ASCII and NUL before any filesystem or database mutation. Keep this
separate from the caller-facing idempotency-key grammar.
Execute the normative DDL against the same matrix: K accepts mixed-case valid R
and rejects every invalid R (including embedded NUL), while the cross-writer
request ledger may remain a wider alphabetic superset but must accept every R.

Run virgin-R initialization separately for local-copy enabled and disabled
direct saves. While K is absent, precreate each of K, request, result, commit,
J, T, U, G, O, S, B, N, C and H; each exact current/stale Tc, V and Xc; each
lexical same-R malformed Tc/V/Xc prefix candidate; and multiple-candidate
forms. The no-transaction preflight must reject without creating L. Race the
same complete matrix into the serialized recheck after O_EXCL L creation; it
may retain only a zero-length L and must not write its nonce or insert K. A prior successful
local-copy-disabled request must already own K, so a later same-key/different-
payload call rejects after proof without any new anchor or artifact mutation.

Run every boundary with prior target absent and present, an identical target,
multibyte bytes, and concurrent exact retries. Assert final target bytes/digest
for uncontested states; collision cases instead assert target plus every pin's
bytes/name set, explicit error, and nonhealthy doctor. Also assert database
counts, stored response, journal/artifact counts, and doctor status.
At every `stage_building` checkpoint use empty, one-byte, every chunk-prefix and
full U. Exact nonce/private-Q/type/uid/0600/nlink proof converges automatically;
wrong nonce/path/inode/uid/mode/type/link is preserved ambiguous. Precreate
wrong-proof U/S, race a second writer (blocked by L), mutate U after IU capture,
and forge wrong bytes at S: only a successful exact private-Q U proof may be
classified as partial; wrong-byte S is never normal crash state.
For the stage nonce, accept only one 128-bit CSPRNG value encoded as 32
lowercase ASCII hex characters and reused exactly in U and the outcome's G/O. Reject 31- and
33-character values, uppercase, every nonhex boundary byte, and mismatched
otherwise-valid U/G/O nonce values or simultaneous G and O.

Race every supported user-target boundary with an independent process. Before B `linkat`,
rename-replace target so B may capture X rather than I0: the target+B postcheck
must reject before exchange or seal, preserve all names, and never classify X as
D0. After a valid B pin but before present-target exchange, replace target with
X: the exchange may move X to S, but remem must already prove N pins D1,
durably record IC and pin the S entry as restore pin C, no-replace evacuate target to H,
then link the restore pin to target without overwriting an intervening create.
Prove X is restored at target and D1 remains pinned, then return
`local_copy_publish_collision` with J/B/S/N/C/H and an unsealed DB.
Also keep the original target FD open after B is linked and overwrite/chmod I0
before exchange, after exchange before `swapped`, immediately before the last
pre-seal verification, after that verification but before COMMIT, after seal,
on both sides of B→O, and after O is durable. No-seal restore keys on structural
I0* identity and retains newer bytes under H/N without sealing when target
choice collides. Matching-seal execution accepts the same structural B/S I0*
drift, may seal exact D1, and must preserve every observed old-D0 write under
permanent O before removing S; it never calls drift tampering.
SIGKILL after exchange before observation; before/after restore-intent, each C
link+fsync, restore-ready, target→H, each C/H→target link, postcheck and P fsync.
Between the final target=N check and target→H, replace target with X: H must
capture X and H→target must restore X no-replace. Between evacuation and publish,
create Y at target: EEXIST must leave Y untouched and retain X/H plus all pins.
Write through FDs opened from the user target after publication, while N already
pins the inode and its later namespace may also name it H/G;
a non-D1 H must be
linked back instead of C. Also write H/N after the exact-D1 decision but before
C→target: require target=C, newer bytes still at H/N, durable
`collision_preserved`, no cleanup/seal, and an explicit latest-not-at-target
diagnostic. Precreate or mutate a remem-reserved name's distinguishable
identity/type/owner/mode/link proof and require security-visible ambiguity.
For any inode already exposed as target, phase-qualified same-inode
mode/bytes/size/mtime/digest drift under B/S/C/H/N/G/O through durable
`cleanup_intent` must be accepted without attributing whether it used an old
target fd or reserved path. After the boundary, continue the same writes through
FDs whose inode is already permanently named by G/O and prove cleanup ignores
only their mutable mode/content fields while retaining exact name/identity/link
proof. Active same-uid nonprotocol mutation inside private Q is outside the
threat model; user-target races remain supported only at the listed boundaries. No trace may
contain recovery exchange or overwrite.

On APFS, separately replace the user target before exchange, after exchange, and
after evacuation. In every case N must already retain D1; the target competitor
must be restored or preserved according to the phase, and no supported
pre-boundary target race may orphan either inode. During uncontested rollback,
write through the old D1 fd opened from target before and after N→G and prove G
exposes the latest bytes. For a sealed prior file, write through the old D0 fd
before/after seal, B→O, both parent fsyncs and S→Xc capture/removal; O must expose the latest
bytes at every point and remain after reconciliation.
For sealed target/N and restored D0 target/B/S/C, inject target
replace/write/chmod before the cleanup snapshot and between its durable fsync
and first revalidation. The former must become collision or be reflected in the
snapshot; the latter must return `local_copy_cleanup_concurrency_violation`
without removing N or the final D0 pin. Repeat before every ordered unlink,
including pre-exchange B cleanup. After successful revalidation the harness
keeps target/nonpermanent pins quiescent through J removal and lock release; a
post-boundary mutation there is a caller-contract violation and is never counted
as preservation evidence. G/O-backed FD writes remain an explicit positive
retention case.
For every ordered name, replace it after the retained proof but before
source→Xc rename: the atomic capture must move the replacement into Xc, detect
the retained-FD/Xc mismatch, restore those replacement bytes to the source
no-replace, and return concurrency without deleting either inode. Separately
write/chmod through the retained source FD after capture: restore Xc and report
drift. Race a new source before mismatch restore; EEXIST must retain both source
and Xc. Assert syscall traces contain no unlink of H/S/B/C/N and no plain
replacement rename—only Xc is unlinked after exact postcapture proof. Across
each successful prefix, assert expected nlink equals all remaining snapshot
aliases for the inode, including G/O, rather than the original snapshot nlink.

Separately race absent-target S→target. Linux `RENAME_NOREPLACE`, macOS
`RENAME_EXCL`, and portable `linkat` must return EEXIST when the competitor
wins, preserve it plus J/S, and leave the DB unsealed. Assert zero target→B
rename, zero plain replacement rename, zero competitor unlink, and no cleanup of
an identity-mismatched B/S in syscall traces. Kill portable publication after
link and before S unlink; exact `{target,S,N}=D1`/nlink=3 must resume to
target/N. After N→G, only target/H both absent is terminal; classify any H
first, and leave an already different target as collision evidence. Race
replacement after observing
target=G but before evacuation: target→H must capture the competitor, then
H→target no-replace restores it or EEXIST preserves both. When H=G, capture and
remove only H through Xc for cleanup source `[H]`; if a new target appears, empty-prefix
revalidation classifies collision and retains J rather than claiming absence.
Kill on both sides of every observation,
rename, classification and P fsync; no trace may unlink the target pathname.

Run a real filesystem probe for readable 0644 and unreadable owner-writable 0200
targets. Write known bytes before chmod; exercise journaled owner-read lift,
double hash, exact mode restoration, B hard-link pin, S↔target exchange and
prepublication N plus C/H no-replace recovery, N→G rollback quarantine and B→O
matching-seal predecessor quarantine. Assert dev/inode,
uid/gid/mode/size/mtime, digest and bytes are preserved; initial
`inspect_intent` must bind only target with B/S/U absent and carry the full
component chain. Source J must carry exact structural `source_namespace`
dev/ino/uid/gid/type/nlink facts and semantic D0/D1 fields before any later read
lift. Target/B then B/S are the exact temporary nlink=2 I0 pair; T/Tc/U, every
mode-qualified V, and each transient Xc remain below private Q, while B/S/C
retain 0644/0200. At restore entry require exact name
sets `D0={target,B,S,C}`/nlink=4 and `D1={H,N}`/nlink=2, then atomically
`D1={H,G}`/nlink=2 and `{G}`/nlink=1, then persist/revalidate
`cleanup_intent` before D0 becomes `(3,1),(2,1),(1,1)` after S/B/C cleanup; G
remains. For the sealed path require D1 `{target,N}` and D0 `{B,S}`, then
`{target,N}` plus `{O,S}` and finally target-only D1 plus permanent O. Unknown
extra links fail closed.
During every unreadable source snapshot and cleanup revalidation, validate J,
bind the pathname and retained write FD to the expected full
dev/ino/uid/gid/type/mode/nlink/size/mtime proof before any marker or chmod, then atomically no-replace
hard-link J→`V=.R.<stage_nonce>.read-lift.<group>.<mode:04o>` and fsync Q before
adding owner-read. Kill after chmod and require exact J/V inode/bytes/nlink=2.
Inject restore failures; retry parses V's group/mode and durably restores it
before V unlink/Q-fsync. Replace target
after the lift while B survives, and separately write through an already-open
target FD; recovery restores through the surviving alias without first
requiring frozen size/mtime/digest, removes V only after durable restoration,
then returns the exact typed concurrency error and nonhealthy doctor state.
With V absent, the same 0200→0600 change is ordinary drift and is not restored.
With V present, change the retained writer from the exact lifted mode to a
third mode immediately before restore. Fresh fstat/re-resolution must reject
without fchmod, leave that mode untouched and keep V armed. Combine callback,
writer-close and finish failures to prove callback identity beats close, while
restoration/finish safety beats callback and retains both diagnostics.
For a 0200 ordered source, retain the reader after V is durably disarmed, capture
source→Xc, and rehash through that FD after both parent fsyncs. Lose held L in
the mode-lift callback: restoration must leave the encoded V and lifted source
recoverable, with no Xc/source deletion. Kill after each capture/restore fsync
and prove restart restores exact 0200 mode and bytes before concurrency or
ordinary cleanup classification.
Exercise D1-link checkpoints. Precreate
B and assert link fails before exchange; probe Linux `RENAME_EXCHANGE` and macOS
`RENAME_SWAP` before target mutation. Unsupported present-target exchange is a
visible no-mutation compatibility error.

Securely create distinct journal Q/locks and Q/quarantine directories at 0700 and a current-uid target
parent P at 0755 on Q's device, including a missing descendant securely created
via its parent dirfd. Prove L/J/T/Tc/V/Xc/U/G/O stay below Q, request-qualified
S/B/N/C/H/target stay below P, and
S inherits IU only after full-D1 atomic no-replace publication with exact
0600/current-uid/regular/nlink=1/entry-FD identity. Reject root/`..` escape,
symlink or
non-directory components, Q alias, wrong parent uid, missing owner rwx,
group/world-writable intermediate or P, cross-device target, changed
component-chain `(dev,ino)` or uid/gid/mode,
and missing directory-fsync, atomic no-replace, or required exchange support.
Replace/rename P after opening its retained dirfd at every path-dependent entry,
callback, mutation, recovery step, and successful return, then recreate the
canonical pathname as empty or decoy. Canonical re-resolution must reject typed
before mutation or success; possession of the old dirfd never authorizes progress,
and neither old P nor replacement P may change except required mode-restoration
unwind through a retained exact FD/alias.
Replace canonical Q after its retained dirfd opens, and independently point
canonical Q at a directory with a decoy mode-0700 `locks`: every capability
check must fail typed lock-unsafe before journal/capture mutation. Preserve the
old Q journal and any new-path victim. Same-uid nonprotocol mutation within the
already validated private Q between a check and Xc unlink is excluded from the
threat model and must not be presented as supported attacker-race evidence.
Place two requests under the same P and prove their `.remem-save-R.*` siblings
are distinct. Supply relative, raw `..`, symlink-ancestor and noncanonical root
paths and require rejection before J. Forge a coherent inode group with one
different digest and add one untracked hard link; both fail before J/chmod.
On first use, crash before/after Q mkdir, Q-parent fsync, each child mkdir,
child fsync, and Q fsync that records `locks` or `quarantine`; restart may use a
directory only after both its own and parent-entry durability proofs pass.

Keep all 23 historical double-crash vectors as named regressions with identical
final outcomes (old reserved+S is now `stage_ready`). Replace the unsafe
B-over-D1 recovery with durable `recover_before_file`: when target=D1 and
B/S are the proved I0 pair and N already pins D1, persist restore intent; accept
absent-C/C prefixes, then target present versus post-evacuation H, desired-C publish,
incumbent-H restore, and EEXIST-target states. For exact D0, enumerate all six
cleanup prefixes across quarantine intent, atomic N→G, H, durable
`cleanup_intent`, S, B, and C cleanup, with P and Q/quarantine fsync at the
defined boundaries. Before
exchange, retain the existing pre-exchange S-then-B cleanup states; retain
absent-target portable three-link, N→G, target→H, H=G cleanup,
H≠G restore/EEXIST plus sealed B→O and cleanup-intent/S/N vectors. For pre-exchange
open-FD drift restart from `(I0*,I0*-link,Ø,Ø)` and `(I0*,Ø,Ø,Ø)`.
For post-exchange drift, kill twice at every restore phase/link/rename/fsync and
prove observed H=N+D1 selects C while any already changed H selects H. Inject
post-choice H/N writes and require target=C plus pinned latest bytes to enter
collision, never cleanup. Add U/D1-link and every source/target race state; no
recovery exchange exists.
Kill/restart at each, then again at every reachable later syscall to a fixed
point. Protocol states converge to D0/absence plus retained G on no-seal after
publication, target D1 plus O for sealed prior-file, or target D1 without O for
sealed prior-absence; competitor states keep the latest bytes at
target or under proved pins and remain visibly unsealed.

Also test canonical only, ordinary T only, source J with Tc
(absent/empty/arbitrary partial/complete), source J with each valid mode-qualified
V, Tc+V, malformed/multiple V candidates, cleanup J with V or stray Tc, cleanup
J with each ordered Xc crash state, source J+Xc, Tc+Xc, V+Xc,
malformed/multiple/stale/unsafe Xc, optimized-mode scanning, the exact retained
locks subtree, completed G/O reported separately from pending artifacts,
distinct G/O names for fresh attempts after completed outcomes, mutation-free
sealed exact replay, no automatic G/O cleanup, unknown phase/name, J goal fields,
every phase×seal cross-product, wrong
pre-exposure T/U/S uid/mode/link count, post-exposure G/O
identity/type/uid/link count, B
identity/metadata/digest drift, inode alias and path escape. Mutate each accepted
physical cell to absent/D0/D1/wrong bytes;
only listed states converge, while every failed proof remains intact/ambiguous.

Tamper separately with journal JSON, nonce/IU/IC/G/O/Xc identity, request fingerprint,
target/backup/U/stage/quarantine path, type, inode alias, ownership/link, and
pre-exposure mode/metadata/digest. Before `cleanup_intent`, separately produce
identical phase-qualified mode/bytes/size/mtime/digest drift on formerly-target
B/S/C/H/N/G/O through an old target fd and its reserved name: both must be
accepted without attribution. Then
remove/invalidate the DB; create an
unexpected combination of target and backup. Every ambiguous case leaves all
user-visible files and journal untouched, returns
`local_copy_reconciliation_ambiguous`, logs at error with only opaque identity,
and exposes `doctor_healthy=false`. Cleanup snapshot mismatch instead returns
`local_copy_cleanup_concurrency_violation` with the same nonhealthy state.
Invoke read-lift begin, finish, restore and recovery with an all-keyword
`intent={"request": R}` call, inject an OS error, and require the exact R (never
`unknown`) in the typed public error under both ordinary and optimized Python.

Inject cleanup permission/fsync failure after commit, plus read-lift restore
fchmod/fsync failures before and after the visible mode returns to 0200 and Xc
capture/restore failures at both parents. The stored `written`
response remains the replay response, pending journal state is durable/visible,
exact retry reconciles before returning, and later doctor cleanup removes only
owned artifacts. V or Xc remains until a fresh restoration fsync succeeds. There is
no warning-only, boolean-only public result, or swallowed cleanup failure.
During both direct revalidation and ordered cleanup, inject the same callback
`AttributeError` and `AssertionError`; each public result is typed
`local_copy_reconciliation_ambiguous` with that exact object as cause, while
B/J remain and no V is created. Repeat under optimized execution.

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
  "held_lock_capability_cases": {},
  "cleanup_capture_cases": {},
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
integrity error, unexpected `local_copy_cleanup_concurrency_violation`, partial
S, competitor overwrite, unexplained forward-only
count, nondeterministic rerun, or
failure outside documented rollback. Passing this rehearsal is necessary but
does not authorize merge or production migration.
