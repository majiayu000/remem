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
`memory_route_tuple_update_guard`, `memory_write_commit_guard`,
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

1. blank/space/uppercase-invalid writer/request identities and short, uppercase,
   nonhex, or overlong fingerprints;
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
   action/status/source/version tuple, Web missing/wrong-type API binding, or a
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
13. a changed route tuple with no staged next row, a sealed staged row, wrong
   head, OLD mismatch, or NEW mismatch; each aborts unchanged, while a matching
   open terminal next row permits the update and null-safe same-value
   assignments need no route row.

For both fingerprint guards, enumerate every table column except row ID/digest
and prove the literal frame has exactly one ordered `old_*` and `new_*` field
(v1 OLD values are typed NULL) plus request fingerprints. Reject v2+ route or
lifecycle `insert|legacy_backfill` and lifecycle `baseline`; insert valid v1/v2,
mutate each OLD/NEW value with reused digest, and only a valid chain may pass.
For `memory_insert_v1_ledgers`, run every insert family, missing/wrong
`insert_origin`, wrong UDF, invalid route/lifecycle value, and injected failure
between its two INSERT statements. A parent INSERT yields exactly memory+route
v1+lifecycle v1 or zero rows—never one ledger. Compare the five literal trigger
bodies independently with normalized `sqlite_schema.sql`.

Exercise anchor DDL independently: only valid opaque R, INTEGER nonnegative dev
and epoch, positive INTEGER ino, and lowercase 128-bit nonce insert. Nonnumeric
TEXT/BLOB/REAL identity values, duplicate R/dev+ino, malformed nonce,
UPDATE/DELETE/OR REPLACE fail unchanged. A short `BEGIN IMMEDIATE` race with
different candidate inodes for one R commits at most one exact anchor. Preseed
another R with the candidate `(dev,ino)` and a zero-length crash-left L; the
combined R-or-IL lookup must reject before nonce write, leaving L bytes/mtime
and every anchor row unchanged.

Explicitly prove that an intent cannot commit without a seal and a seal cannot
commit without all manifested results. `PRAGMA foreign_key_check` is empty after
every positive case.

## Writer and Retry Matrix

Exercise the six current memory insert families and the three existing-row
transition writers named in `TECH.md`. For each, assert intent precedes mutation,
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
| `sealed`; prior-file `predecessor_quarantine_intent`; B→O no-replace; P and Q/quarantine fsync; `predecessor_quarantined` | not reachable without seal | target/N keep D1; exactly B or O plus S keeps D0 across every crash; O becomes permanent before S cleanup and receives all late old-D0-FD writes. Prior absence creates no O |
| `cleanup_intent` source-J/temp-J transition; snapshot/J fsync; post-persist revalidation; every ordered unlink/parent fsync and empty-prefix revalidation | exercise exact `[B]`, `[H,S,B,C]`, and `[H]` sources | exercise exact `[S,N]` and `[N]` sources. Before the boundary, injected target replacement/write/chmod is retained or collides; snapshot-to-revalidation mismatch returns `local_copy_cleanup_concurrency_violation` with J/pins. After successful revalidation target/nonpermanent pins stay quiescent; G/O-backed mode/content drift remains allowed and every crash resumes the exact prefix |
| recovery-phase fsync and every C link, present/absent target→H rename, C/H→target link/rename, N→G or B→O rename, owned unlink/fdatasync and P/Q-quarantine fsync | restart the same phase/prefix and converge without pathname target unlink, overwrite, or pre-boundary loss of the last D1 name | restart same phase and converge with old D0 permanently under O |
| journal unlink/directory fsync | keep exact after digest; no journal | same |

At every table boundary and both sides of each listed syscall, use separate OS
processes, not threads: the writer holds the anchor-verified retained L inode while a startup
scanner and a doctor/reconciler each attempt that exact lock. Cover pre-J,
`inspect_intent`, `reserved`, `stage_building`, every U create/write/fsync,
`stage_ready`, U→S, `staged`, `swap_intent`, B-link before/after `backed_up`,
`new_pin_intent`, S→N, `new_pinned`, `exchange_intent` fsync,
present-target exchange/restore pins or absent-target
publish before/after `swapped`, every DB
result and seal step, COMMIT-before-`sealed`, `sealed`,
`predecessor_quarantine_intent`, B→O, `predecessor_quarantined`, `cleanup_intent`,
each cleanup/J fsync, and
each recovery phase/action. Both scanner/doctor contenders must return
`local_copy_writer_in_progress`, and an event trace
must show no J/U/G/O/S/B/N/C/H/target open, classification or mutation and no R-scoped DB
read/write except the immutable K lookup needed after candidate acquisition.
Snapshot bytes, inode, digest and phase must remain unchanged.
Enumerate every closed J phase against no seal, its matching seal, and a
mismatched seal. The only COMMIT-before-J-`sealed` positive case is
`swapped` plus matching seal; `sealed`, predecessor-quarantine and recover-after
require matching seal, recover-before/restore/rollback-quarantine/collision
require none, and `cleanup_intent` admits only its five exact source tuples.
Reject all other
cross-products, any unknown phase, and any J `goal` or goal-like field.
For each cleanup source, positively prove its exact `before_kind`, seal state,
namespace/nlink snapshot and ordered list: preexchange file/no-seal `[B]`,
rollback file/no-seal `[H,S,B,C]`, predecessor-quarantined file/matching-seal
`[S,N]`, after-absent/matching-seal `[N]`, and rollback absent/no-seal `[H]`.
Permute or duplicate the list, substitute any source/`before_kind`/seal/name,
or add/remove an entry and require no mutation. Crash the source-J→T→J
transition at each write/fdatasync/rename/Q-fsync: accept only source J plus a
complete exact cleanup T or canonical cleanup J with T absent/identical.
Explicitly hold the writer with durable D1 at S/N and at target/N but no DB seal;
doctor must neither restore D0 nor delete D1/J/U/G/O/S/B/N/C/H.
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
scanner/doctor missing K. Reject alternate paths, process-local mutexes and
PID/mtime inference; wrong lock/locks-dir identity/type/uid/mode/link count/content
or anchor mutation/mismatch must fail after at most K lookup.

Run virgin-R initialization separately for local-copy enabled and disabled
direct saves. Precreate each of K, request, result, commit, J, T, every valid U,
G or O grammar name, S, B, N, C, and H while K is absent: the no-transaction preflight
must reject without creating L. Race each state into the serialized recheck
after O_EXCL L creation; it may retain only a zero-length L and must not write
its nonce or insert K. A prior successful
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
before exchange, after exchange, before seal, after seal, on both sides of
B→O, and after O is durable. No-seal restore must key on stable entry identity,
return observed bytes through H or C when stably selected, otherwise retain
newer bytes under H/N and never seal. Matching-seal recovery must preserve every
observed old-D0 write under permanent O before it removes S; it must never
misclassify drift as tampering or discard it.
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
proof. Active malicious reserved unlink is outside the contract. No trace may
contain recovery exchange or overwrite.

On APFS, separately replace the user target before exchange, after exchange, and
after evacuation. In every case N must already retain D1; the target competitor
must be restored or preserved according to the phase, and no supported
pre-boundary target race may orphan either inode. During uncontested rollback,
write through the old D1 fd opened from target before and after N→G and prove G
exposes the latest bytes. For a sealed prior file, write through the old D0 fd
before/after seal, B→O, both parent fsyncs and S unlink; O must expose the latest
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
H→target no-replace restores it or EEXIST preserves both. When H=G, unlink only
H through cleanup source `[H]`; if a new target appears, empty-prefix
revalidation classifies collision and retains J rather than claiming absence.
Kill on both sides of every observation,
rename, classification and P fsync; no trace may unlink the target pathname.

Run a real filesystem probe for readable 0644 and unreadable owner-writable 0200
targets. Write known bytes before chmod; exercise journaled owner-read lift,
double hash, exact mode restoration, B hard-link pin, S↔target exchange and
prepublication N plus C/H no-replace recovery, N→G rollback quarantine and B→O
matching-seal predecessor quarantine. Assert dev/inode, uid/gid/mode/size/mtime, digest and bytes are
preserved; target/B then B/S are the exact temporary nlink=2 I0 pair, T/U remain
private-Q 0600, and B/S/C retain 0644/0200. At restore entry require exact name
sets `D0={target,B,S,C}`/nlink=4 and `D1={H,N}`/nlink=2, then atomically
`D1={H,G}`/nlink=2 and `{G}`/nlink=1, then persist/revalidate
`cleanup_intent` before D0 becomes `(3,1),(2,1),(1,1)` after S/B/C cleanup; G
remains. For the sealed path require D1 `{target,N}` and D0 `{B,S}`, then
`{target,N}` plus `{O,S}` and finally target-only D1 plus permanent O. Unknown
extra links fail closed.
Exercise D1-link checkpoints. Precreate
B and assert link fails before exchange; probe Linux `RENAME_EXCHANGE` and macOS
`RENAME_SWAP` before target mutation. Unsupported present-target exchange is a
visible no-mutation compatibility error.

Securely create distinct journal Q/locks and Q/quarantine directories at 0700 and a current-uid target
parent P at 0755 on Q's device, including a missing descendant securely created
via its parent dirfd. Prove L/J/T/U/G/O stay below Q, S/B/N/C/H/target stay below P, and
S inherits IU only after full-D1 atomic no-replace publication with exact
0600/current-uid/regular/nlink=1/entry-FD identity. Reject root/`..` escape,
symlink or
non-directory components, Q alias, wrong parent uid, missing owner rwx,
group/world-writable P, cross-device target, changed `(dev,ino)` or uid/gid/mode,
and missing directory-fsync, atomic no-replace, or required exchange support. Replace/rename the
parent after opening its dirfd at each revalidation boundary: operations must
remain bound to the proved P and publish nothing at the replacement path, or
fail visibly without mutation.
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

Also test canonical only, temp only (empty/partial/complete), both, the expected
retained locks subtree, completed G/O reported separately from pending artifacts,
distinct G/O names for fresh attempts after completed outcomes, mutation-free
sealed exact replay, no automatic G/O cleanup, unknown phase/name, J goal fields,
every phase×seal cross-product, wrong
pre-exposure T/U/S uid/mode/link count, post-exposure G/O
identity/type/uid/link count, B
identity/metadata/digest drift, inode alias and path escape. Mutate each accepted
physical cell to absent/D0/D1/wrong bytes;
only listed states converge, while every failed proof remains intact/ambiguous.

Tamper separately with journal JSON, nonce/IU/IC/G/O identity, request fingerprint,
target/backup/U/stage/quarantine path, type, inode alias, ownership/link, and
pre-exposure mode/metadata/digest. Before `cleanup_intent`, separately produce
identical phase-qualified mode/bytes/size/mtime/digest drift on formerly-target
B/S/C/H/N/G/O through an old target fd and its reserved name: both must be
accepted without attribution. Then
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
integrity error, unexpected `local_copy_cleanup_concurrency_violation`, partial
S, competitor overwrite, unexplained forward-only
count, nondeterministic rerun, or
failure outside documented rollback. Passing this rehearsal is necessary but
does not authorize merge or production migration.
