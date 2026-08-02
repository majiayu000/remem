# GH933 Product Contract — CurrentTruth Projection

Refs #933.

## Status

Phase A v1 was merged in PR #939 and publicly released in `remem-ai` 0.6.26;
0.6.27 still contains that API. It is a library-only baseline, not completion
of GH-933.

This document defines the pending Phase A v2 hardening contract. Its checkboxes
remain open until implementation and fresh verification land. Phase A now
includes narrow durable route/lifecycle ledgers, migration/backfill, and writer
instrumentation required for exact history. Phase B Context Bundle consumption,
worktree/task scope, and Phase C general writer convergence remain later work,
so GH-933 stays open.
This Phase A v2 persistence/cutover contract is Unix-only until a separately
reviewed Windows-native locking/publication contract lands; supported Windows
installations remain on the existing v1 runtime and are never silently migrated.

The issue packet under `specs/GH933/` is historical planning evidence. This
five-file `docs/specs/GH933/` set is the normative current contract.
`MIGRATION-CUTOVER.md` defines executable persistence/recovery;
`MIGRATION-REHEARSAL.md` and `ROLLOUT.md` define proof and human gates.

## Problem

remem stores evidence, memories, observations, user-context claims and graph
relations with different status, scope and time semantics. Callers need one
deterministic answer to:

> At a given reference time and scope, which claims currently stand, which
> conflict, and which evidence supports the result?

The v1 baseline exposes this projection but has correctness gaps around
canonical subject identity, historical mutation, Observation mapping, policy
suppression, external-content trust, relation bounds and reference-time
replayability.

## Current v1 Evidence

- [x] Public `remem::truth` module with versioned read DTOs.
- [x] Memory and user-context Claim adapters.
- [x] Captured-event/source-ref Evidence adapters.
- [x] Memory/graph/user-supersedes Relation adapters.
- [x] Deterministic supersedes/refutes/trust/recency resolver.
- [x] Explicit conflict and abstention results.
- [x] SELECT-only design intent and 18 baseline truth tests.

These checks describe released v1 evidence only; they do not satisfy the
pending v2 requirements below.

## Phase A v2 Behavior

1. **Typed subject identity.** Memory identity includes source, canonical
   owner pair, normalized memory scope, memory type and a nonempty topic key.
   Only a NULL or exact-empty topic key is a `memory:<id>` singleton; nonempty
   keys, including whitespace, remain byte-exact. User claim identity includes
   exact owner, claim type and claim key. Different owners, scopes, types or
   singleton rows never compete.

2. **Explicit query scope.** Project scope supports branch-neutral + exact
   branch queries; a missing branch remains the v1 branch-agnostic all-branch
   view. Project memory inclusion follows canonical repo owner/target routing,
   but every full or legacy arm excludes normalized global scope; owner-null
   placement is only the non-global legacy fallback. Stale placement cannot
   leak a non-repo reroute. Owner scope
   selects exact-owned memories plus exact-owned user-context claims across all
   branches. Global memories are Owner-scoped, not ambient Project rows. A
   selector is exact: Owner requires that owner; Project membership follows
   routing + branch and can include owner Q via `target_project=P`. A compatible
   selector with no row yields an empty truth list, not synthesized Unknown.
   Owner scope is exactly user/workspace/repo/tool/domain/workstream/session,
   its paired key is nonblank/ASCII-trim-stable, and partial pairs fail integrity.
   Memory-scope is closed; trimmed `" global "` is global.
   Explicit history discovers candidates from a persistent, scope-indexed route
   ledger, then reconstructs owner, target, versioned memory scope, memory type
   and the raw nullable topic key at the cutoff from its complete version chain.
   NULL and exact-empty topic keys stay distinct in the ledger even though both
   map to the same `memory:<id>` singleton rule. A trigger covers every creation/
   import. The three existing-row writers—normal save upsert, Markdown restore/
   import, and scope cleanup—use one canonical route-transition service to
   atomically append a route version whenever their actual placement/branch/
   scope/source/target/owner/memory-type/topic-key/topic-domain/routing/context
   tuple changes; same-value assignments remain legal no-ops. Markdown transitions use
   `source_kind=markdown_import`; scope cleanup also appends its same-status
   lifecycle version and audit mirror. A guard rejects every other direct change,
   including reuse of a sealed staged row; seal itself requires terminal route
   and lifecycle snapshots to exact-match the current memory.
   A validated A→B→C chain remains discoverable in B. Because legacy save and
   Markdown mutations were not exhaustively logged and 30-day audit events may
   be gone, migration marks history complete only with exhaustive durable proof;
   otherwise it starts forward-only coverage at migration. A pre-floor query
   fails with `unreconstructable_routing_history` before scope filtering.
   Project/Owner membership and SubjectIdentity use the full route-at-t,
   including scope, with the new route effective at transition equality. A
   validated Markdown project→global transition is Project-scoped before and
   Owner-scoped at/after equality. Only missing, discontinuous, contradictory,
   invalid-scope or legacy/forward-only coverage-gapped history returns
   `unreconstructable_routing_history`; a validated scope change does not.

3. **Auditable reference time and snapshot.** An explicit `as_of` is used
   directly and is `Exact`. A query without one samples “now” once. Every output
   serializes the requested value, effective `reference_epoch` and replayability.
   A current result depending on an operation-less binding or an unversioned
   entity link is `CurrentSnapshotOnly`, not replayable merely by passing the
   sampled epoch. All reads share one epoch and one SQLite read snapshot.

4. **Temporal correctness.** Source time and remem knowledge time must both be
   no later than the reference epoch. A valid user-claim edit chain restores
   the old version before its transition. ClaimView state time and immutable
   provenance-root SourceRef binding remain distinct, so a successor cannot
   rebind inherited refs or erase pre-transition evidence. Candidate replacement and
   no-op application reconstruct all writer-superseded same-identity
   co-predecessors even though only one row may have an explicit successor
   link. In-place suppress/unsuppress/delete mutations without a historical row
   are conservatively excluded/Unknown; current bytes must not be projected
   backward. Every production memory-status change is reconstructed from one
   durable, memory-indexed lifecycle ledger. This includes `govern_memories`, Web
   archive/restore, scope archive/reroute/cleanup-plan, save/Markdown reactivation,
   candidate application, TTL expiry, soft supersede, preference removal, and
   stale archive. One canonical service commits status and the next changed
   version atomically; a database guard rejects any real status update without
   its open exact staged successor, and same-status writer rows are forbidden.
   Web versions bind the durable API operation record, while audit events remain
   optional mirrors. No v2 startup enables an uninstrumented path.
   Both history ledgers are retained indefinitely with restrictive memory/self
   links and no FK/cascade to the 30-day `events` table.
   Every append has a nonblank canonical writer-specific request discriminator
   and strict lowercase 64-hex SHA-256 transition fingerprint over the ledger
   domain/version, memory, predecessor, request hash, result ordinal and exact
   typed OLD/NEW state. The write connection registers the versioned deterministic
   `remem_sha256_frame_v1` SQLite function before migration or mutation; a
   missing/wrong function fails the INSERT rather than weakening the hash.
   Before any mutation, each writer appends an immutable request intent with an
   exact typed-result manifest. Generated memory, ledger, audit and operation IDs
   are outputs. Every successful transaction fills that manifest, then appends
   the immutable final response/result seal. Ledger INSERT requires an open
   request and compatible manifest slot; sealing reverses every ledger row to
   its manifest-declared typed result, including exact integer/API operation and
   audit provenance, and seal blocks every later append.
   Deferred constraints reject unsealed/missing/extra/mismatched bindings.
   Anchor, intent, result, seal and ledger rows reject UPDATE, DELETE, and every
   conflict-path `INSERT OR REPLACE` even with recursive triggers disabled.
   Every internal/API writer/request identity, nonce, SHA-256, and digest requires SQLite TEXT
   storage and no embedded NUL; every integer-domain ledger value requires
   INTEGER storage. BLOB and NUL-tailed prefix bypasses must fail.
   Caller-facing save requires an explicit `idempotency_key`. Adapters validate
   it, derive a namespaced opaque request ID and never persist or log the raw key.
   The key and transport credentials are identity, not request payload, so the
   request fingerprint covers every other raw `SaveMemoryRequest` value,
   deterministic defaults and exact effective adapter inputs—including
   local-copy, claim and poisoning-ack options—without generic newline/trim
   folding. Same key plus equal payload replays; same key plus different payload
   conflicts; a different key with byte-identical payload is a distinct write
   and preserves intentional lesson reinforcement. Markdown uses a stable source
   or canonical no-source archive identity and remains stable across its
   importer-owned metadata rewrite.
   Every direct save, even with local copy disabled, serializes its request on
   the same retained OS-visible exclusive lock used by the startup scanner and
   doctor/reconciler. A candidate lock becomes a protocol owner only after a
   short transaction exact-matches its fd/path inode and file nonce to an
   immutable database anchor; path replacement therefore cannot create a second
   verified owner. An absent anchor is initialized only after a bounded
   existence-only proof that the request and all exact artifacts are virgin;
   the serialized transaction also rejects an inode already anchored to another
   request before writing the lock nonce. Any old state fails closed rather than
   being re-anchored. The writer proves
   this before request lookup or mutation and holds the lock through seal plus
   cleanup/reconciliation. An enabled local copy additionally locks the canonical
   target-path digest and durably binds its current request owner before target
   inspection; request→target is the only lock order, and both remain held through
   postcommit journal/pin cleanup and owner removal. A different request for the
   same target first reconciles the recorded owner, while distinct targets may run
   concurrently. Local-copy mutation then uses a fsynced write-ahead journal.
   Internal local-copy request `R` is exactly 1–128 ASCII bytes matching
   `[A-Za-z0-9][A-Za-z0-9_-]{0,127}`; this namespace grammar is distinct from
   validation of the caller's raw idempotency key. Because POSIX record locks
   are process-scoped and closing another FD for L can release the process's
   lock, acquisition atomically reserves `(Q/locks dev, Q/locks ino, R)` in a
   process-local registry before opening L. A same-process duplicate therefore
   fails without opening or closing L; the registry supplements rather than
   replaces the kernel lock. Before a forked child can acquire anything, its
   callback moves every inherited capability FD out of its owner, invalidates
   capability state, makes exactly one close attempt per FD, and resets the
   registry. No numeric FD is probed or retried after close returns, including on
   error, so an old child object cannot close a recycled FD or release a fresh
   child lock. Acquisition returns
   a mandatory, request- and PID-bound held-lock capability that retains the exact
   `Q/locks` directory FD even after the caller closes its original FD. Every
   inspection, source/cleanup-journal transition, read lift, snapshot proof,
   cleanup capture, recovery, ordered cleanup, and J removal accepts that
   capability with no unlocked/default path. Entry, callback, rename, unlink,
   and journal boundaries reprove the live kernel lock, immutable L identity,
   canonical no-follow Q identity, and canonical `Q/locks` binding. Independently,
   every path-dependent inspection, read-lift, snapshot, journal, or cleanup step
   reopens the canonical trusted-root→P chain and exact-matches retained P before
   mutation, recovery, and successful return; the retained P FD alone is never
   authorization after its canonical binding changes. A closed,
   fork-inherited, wrong-request, decoy-locks, replaced-Q, or replaced-L
   capability fails visibly before mutation. L is never unlinked; normal close
   or process death releases only the kernel lock on the same retained inode.
   A direct save retries only lock acquisition for
   at most 5 seconds, then either acquires the lock and replays the sealed winner
   or reports the still-live writer; scanner/doctor contenders report immediately.
   No contender inspects or recovers request artifacts while unlocked. A journal-durable random-nonce `stage_building`
   phase writes empty/partial bytes only to a proved 0600 build file below the
   private journal root. Only its fdatasynced D1 inode is atomically published
   no-replace as stage below the separately verified target parent, so partial S
   is impossible. Before any target publication, durable `new_pin_intent` and
   `new_pinned` phases no-replace hard-link that stage to N, prove S/N are the
   complete two-name D1 set, and fsync the target parent. For an existing
   single-link target, a no-replace hard link
   first pins the reverified original inode as backup, retaining identity,
   metadata, digest and legal permissions such as 0200 across crash-safe
   inspection and cleanup revalidation. A durable
   `exchange_intent` precedes atomic stage/target exchange and accepts its exact
   before, normal-after, captured replacement, or same-original-inode in-place-
   write crash tuple; only then does the writer prove target+N are exact D1 and
   backup+stage are the same structural predecessor I0*, recording rather than
   rejecting phase-qualified mutable drift for permanent retention under O.
   An unsupported exchange fails before target mutation. Recovery never reverses
   the exchange. N already pins D1, so recovery pins only the restore entry,
   no-replace evacuates whichever entry is currently at target into H, and
   no-replace links the restore pin only while H is observed as exact D1;
   otherwise it links H. A target create during the brief absence wins with
   EEXIST and is untouched. An open-FD write between the check and link cannot be
   content-CASed portably: target may hold the restore entry while newer bytes
   remain named by H/N. That visible collision retains every pin, stays unsealed
   and keeps doctor nonhealthy; it never claims latest bytes are at target.
   For an uncontested no-seal rollback, N is atomically renamed no-replace to a
   nonce-qualified file G in the private same-device quarantine directory,
   both parent directories are fsynced, and G is retained indefinitely before H
   or a target-derived D1 name is removed. Late writes through an already-open
   D1 fd therefore remain visible through G. For a prior-absent rollback,
   recovery never unlinks the target pathname. After G is durable, absence is
   terminal only when both target and H are absent; an existing H is first
   classified as G/D1 or a competing inode and is removed, restored no-replace,
   or retained on EEXIST accordingly. A target already different from G is
   collision evidence. Only an observed target=G is atomically evacuated to H
   no-replace. Thus recovery restores D0/absence only when uncontested while
   retaining G. A seal keeps D1 at target. When a prior file existed, sealed
   cleanup first atomically renames its proved B pin no-replace to nonce-qualified
   O in the private quarantine, fsyncs both parents, and retains O indefinitely;
   old-D0-FD writes therefore remain named even after S is removed. A
   prior-absent seal has no O.
   Before cleanup can remove the final protocol pin of any inode formerly
   exposed as target, it persists and fsyncs `cleanup_intent` with the source
   phase, exact frozen namespace/name/identity and cleanup-relevant
   metadata/digest snapshot, and one source-authorized ordered unlink list.
   Before that transition, the canonical source J records the request/stage
   fingerprints, epoch, source phase, prior-kind/publication/seal state,
   `semantic_d0_digest`/`semantic_d1_digest`, exact path/component proofs, and
   `source_namespace` entries containing only dev/inode/uid/gid/type/nlink.
   Mutable mode/size/mtime/digest are first frozen in `cleanup_intent`.
   Cleanup conversion uses a nonce-qualified private `Tc`, distinct from the
   ordinary phase-update temp. A request-wide scanner reserves every same-R
   cleanup-temp candidate across nonce values, including malformed basenames.
   While source J is canonical, only the sole exact current-nonce Tc may have
   provisional bytes discarded and Q-fsynced or, when byte-exact to the fresh
   complete candidate, replace J. Stale-nonce, malformed, or multiple candidates
   are preserved ambiguous, and canonical cleanup J rejects every same-R `Tc`.
   An unreadable source or cleanup snapshot first exact-matches its pathname and
   retained write FD to the cleanup snapshot's full raw proof before it arms the
   nonce-qualified hard-link
   `V=Q/.R.<stage_nonce>.read-lift.<group>.<mode:04o>` to canonical J and fsyncs
   Q before adding owner-read; V may coexist with Tc. A request-wide scanner
   reserves every same-R read-lift candidate across nonce values; only one exact
   current-nonce basename is usable, while stale, malformed—including a same-R
   lexical prefix with an extra dot—or multiple V names are preserved ambiguous;
   a distinct valid request prefix remains isolated. Its basename binds the source group and observed original mode. Recovery restores and fsyncs that
   mode through any surviving exact structural alias before removing V; failure
   retains V, and an unmarked single-read-bit change remains ordinary drift.
   Recovery derives the snapshot, removed prefix, and exempt identities only
   from V's persisted canonical J plus the current namespace; caller-supplied
   transient values must exact-match that derivation or V/J remain untouched.
   Read proof and V begin likewise derive the canonical namespace, group, and
   observed mode, while V finish independently proves that a surviving exact
   alias has regained the encoded mode before it can remove the marker. Inspection
   owner-read restoration and V begin/finish/restore/recovery re-resolve canonical
   P before and after every path-dependent mutation and before every success return.
   Each public capture or restart entry independently requires the exact
   canonical `cleanup_intent` J, its exact field set, trusted-root/directory-handle
   proofs and logical path bindings, its allowlisted contract, no forbidden
   V/Tc/Xc coexistence, and the exact next ordered member: every prior member is absent
   and every later member remains present. Each ordered removal then retains its verified reader/proof and atomically
   renames the source no-replace to
   `Xc=Q/.R.<stage_nonce>.cleanup-capture.<H|S|B|C|N>`. It fsyncs Q and then the
   source parent, revalidates held L, proves the source name absent, and
   rechecks both Xc and the retained FD/digest. Exact capture is removed only by
   unlinking Xc followed by Q fsync. Any mismatch is restored Xc→source
   no-replace and durably fsynced source-parent then Q; EEXIST preserves both
   names and returns the cleanup-concurrency error. Restart restores a sole
   valid Xc before deriving the removed prefix, so a crash cannot turn a
   captured entry into a completed unlink. Malformed, multiple, stale-nonce,
   symlink, wrong-owner/type/device, or out-of-order Xc is preserved ambiguous;
   unsupported native no-replace rename fails before source mutation. V+Tc is
   the sole allowed marker/temp coexistence; V+Xc and Tc+Xc are ambiguous.
   Immediately before J unlink, the public boundary again validates canonical J,
   the full intent/path contract, held L, and request-wide absence of every Tc,
   V, and Xc candidate. It independently proves every ordered cleanup name absent
   and every retained target/G/O or required absence against the terminal snapshot
   with runtime nlink; an early direct call or late stale artifact keeps J and all
   remaining names intact with typed ambiguity.
   Snapshot nlink is not reused after the first removal: each capture expects
   the number of still-named snapshot aliases for that inode, including any
   permanent G/O alias.
   Every inode that may receive user bytes remains named by the user target or
   permanent G/O; every remaining prefix is revalidated before the next unlink
   and once more before J removal. Target replace/write/chmod and unretained
   old-target-FD activity are supported through the durable boundary. From that
   boundary until J removal and lock release, the caller must keep the target
   and nonpermanent pins quiescent; detectable drift returns
   `local_copy_cleanup_concurrency_violation`, preserves every remaining pin
   and J, and keeps doctor nonhealthy. Post-boundary activity on those entries
   violates this contract and has no preservation guarantee; phase-qualified
   G/O content or mode drift remains safe because those names are permanent.
   Every phase proves exact name sets and is restartable across all required
   directory fsyncs. Journal,
   quarantine, and request-qualified `.remem-save-R.*` names are remem-reserved; distinguishable
   identity/name/type/ownership/link tampering fails closed and security-visible,
   while active nonprotocol mutation by the same uid inside private mode-0700 Q
   (including a check-to-Xc-unlink substitution) is outside the threat model.
   Canonical-Q or `Q/locks` replacement observed at any validation boundary is
   nevertheless a typed lock-unsafe abort that mutates neither the retained old
   Q nor the replacement. For an inode already
   exposed as user target, phase-qualified same-inode mode/bytes/size/mtime/digest
   drift is accepted wherever the protocol now names it B/S/C/H/N/G/O: an old
   target-FD operation and a direct reserved-path operation are physically
   indistinguishable and cannot be attributed portably. Every unlisted state
   fails closed untouched.
   An uncontested crash before commit leaves no committed database state and
   restores the user target, with any published D1 retained privately as G for
   recovery evidence. A collision instead retains J and every proved pin and
   may intentionally leave the target unrestored. After commit/response loss,
   an exact-key retry returns the
   committed winner without another file write, version, event, operation, claim,
   or knowledge epoch. Same-second transitions remain predecessor/version ordered.
   Every previous status must equal the prior new status and the terminal status
   must equal the current row. The new status is effective at transition
   equality; an unsupported/unrecorded transition, gap, fork or contradiction returns
   `unreconstructable_memory_lifecycle`. Because backup/Markdown imports preserve source timestamps,
   `updated_at_epoch` alone is not ingestion proof: the earliest
   route-at-operation-compatible canonical result operation, candidate completion and
   validated acknowledgement define memory knowledge; an unproven memory is
   current-snapshot-only and cannot enter explicit historical results. This
   covers canonical procedure memories that currently lack an operation record.
   After ingestion, a canonical no-op with validated result provenance records
   later trust/ack rewrites and advances knowledge time but cannot prove initial
   ingestion; its request topic may legitimately differ from the result topic.
   Candidate completion is validated once against its initial route-ledger
   state, including owner/project/scope/type/raw topic key. Every later read
   folds the complete chain to the query cutoff for membership and emitted
   `SubjectIdentity`; it does not recompare the immutable candidate identity
   against that later state or today’s row. Thus a proved
   owner, project, scope, memory-type or topic-key transition is legal; the
   terminal snapshot must still equal the current row. Missing/discontinuous/
   forward-only history returns the routing-history error, while an unexplained
   route mutation or content/provenance drift fails closed.
   Captured-event identity `(host_id, session_id, event_id)` is immutable across
   idempotent replay: a duplicate cannot replace its original creation,
   insertion/knowledge or reference/source epoch. Replay may append separately
   keyed Git evidence or extraction work. Existing pre-v2 rows keep their stored
   insertion epoch as a conservative knowledge floor; v2 never backdates
   eligibility it cannot reconstruct.

5. **Evidence trust without escalation.** Captured events reuse canonical
   `SourceTrustClass` semantics. WebFetch/WebSearch, `mcp__*`, network-fetching
   Bash and pack/external content cannot become Verified merely because a tool
   produced them. The truth read side reconstructs canonical `raw_keep` inline
   content at the exact 16,384-byte boundary or a verified plain `raw_compact`
   blob above it, including valid legacy hashes, then calls the canonical pure
   classifier. It never classifies only the stored preview; invalid storage,
   UTF-8, lengths, preview or hashes fail closed. Phase A may expose pure capture
   helpers and the classifier; its only capture-writer correction makes duplicate
   event timestamps immutable without changing payload semantics. Effective trust
   is the weaker of the strongest eligible evidence and a cap formed from both
   the stored class and the weakest canonical reclassification of all referenced
   events. This protects legacy
   rows whose stored default is too strong. SourceTrustClass diagnostic
   evidence never participates in the strongest-evidence max, and a cap cannot
   uplift a claim that lacks verified evidence. Candidate-backed memory also
   retains the candidate’s source cap after later memory trust rewrites.
   Candidate-derived claims must
   resolve an authoritative candidate/result row, exact persisted copied fields
   plus route-derived memory scope (not candidate input scope or derived title) and
   nested provenance, binding-time-eligible referents and a valid edit chain;
   wrapper IDs or later-created sources cannot launder evidence. Explicit user
   statements require at least one first-party user event. Memory/Observation
   refs must have source/knowledge epochs no later than immutable binding and
   share canonical provenance; candidates and nonlegacy Observations also bind
   exact host/project/session identity. Epoch equality is eligible because the
   schema has second resolution, so Phase A cannot distinguish later writes in
   the same second; a durable attachment sequence is a Phase C prerequisite
   for stricter ordering. Nested SourceRef IDs use stable canonical paths.

6. **Observation mapping.** Active observations appear as versioned Evidence
   in a stable `evidence_catalog`, with canonical
   `observation:<id>`, project/branch, lifecycle, source/knowledge times and
   validated captured-event refs. NULL refs mean no refs; non-null refs are
   strict; a NULL creation epoch is an integrity error, not a fabricated time.
   Active historical rows are re-scanned before they can be marked Validated.
   Observation trust defaults to ModelGenerated when refs are empty and falls to
   Untrusted when any canonical supporting source is external. Observation
   never becomes a Claim. Attachment to a memory Claim requires a scoped,
   bitemporally effective `memory_facts` row that explicitly contains both
   `source_memory_id` and `source_observation_id`; both its caller-supplied
   learned time and actual NOT-NULL insertion `created_at_epoch` must be no later
   than the cutoff, with no legacy timestamp fallback.
   Current queries exclude rows whose current lifecycle is stale/compressed.
   Explicit history returns a contextual integrity error when such a scoped row
   existed by the cutoff but lacks complete validated transition history; it
   cannot silently drop evidence or change a winner using current status.

7. **Quarantine safety.** `poisoning_quarantined` maps to
   Candidate/Unknown/Live/Suppressed and is excluded before the usable catalog,
   claim attachment, trust aggregation and current truth. Unknown stored status
   values fail closed. No immutable link identifies a summary’s complete
   generated-surface set, so every structured session-summary ref fails with
   `unverifiable_session_summary_provenance` before content/trust use. Status or
   acknowledgement cannot make it usable in read-only Phase A.

8. **Policy suppression.** Memory-ID/topic/entity/pattern and user-claim
   targets use the exact ID/equality/substring matrix and change Visibility,
   not validity. Canonical `user_context_candidates` and
   `user_context_summaries` targets validate exact ID/value shapes but remain
   non-applicable; they do not transitively hide promoted claims or evidence.
   `(NULL,NULL)` owner means global; a complete owner pair is exact; a partial
   pair is an error. Historical queries honor creation and revocation
   boundaries, including visibility restoration at revocation equality.
   Entity membership itself is unversioned: current evaluation is
   `CurrentSnapshotOnly`, while an effective entity-target suppression in an
   explicit historical query fails with `unreconstructable_entity_link_history`
   unless durable link history proves membership and non-membership.

9. **Scoped relations.** Both endpoints must be in the scoped claim set.
   Supersedes and ordinary Refutes only decide within one exact identity.
   Scoped Supports/DerivedFrom provenance may cross identities without changing
   the winner. Unbacked means both source IDs are NULL. Candidate-only provenance
   is invalid; operation-only and candidate+operation rows follow their exact
   validation paths. Every scoped `memory_edges.edge_type` is parsed through the
   closed six-kind writer domain; an unknown/newer/typo value is a contextual
   table/edge/raw-value error, never a silently omitted relation.

10. **Canonical preference conflicts.** A validated operation-backed conflict
    can mark two same-owner/scope/branch preference survivors Contradicted
    across topic slots. Structurally valid canonical heterogeneous pairwise
    conflicts remain decision-neutral; malformed claimed operation provenance
    is a contextual error. Candidate completion defines relation knowledge, and
    both outputs use the deterministic full-field shape in `TECH.md`. Uniform
    conflict pairs must form a matching; an A-B plus A-C survivor graph errors.

11. **Bounded reads.** Historical candidate discovery uses the route ledger's
    owner, target and legacy-placement indexes; route/lifecycle chains, raw
    edges, evidence, facts and suppression use scoped/indexed SQL and stable ID
    chunks of at most 900 without scanning `events`. Large unrelated
    projects do not change target query counts, returned rows or output. The full
    projection runs in one deferred SQLite read snapshot; transaction control is
    read orchestration, not a canonical-data write.

12. **Deterministic output.** DTO serde shape, truth/evidence/relation ordering,
    deduplication and v1→v2 golden differences are explicit. No LLM, network,
    migration or write occurs during projection. Missing data yields an empty
    result; Unknown/abstention requires a loaded identity with no survivor.
    Malformed state yields a visible error, not silent degradation.

## Pending Phase A v2 Acceptance

- [ ] Public v2 DTO/selector/output matches `TECH.md`, reports
      `projection_version=2`, and compiles through `use remem::truth`.
- [ ] Typed owner/scope/type identity, repo owner/target Project routing,
      non-repo reroute exclusion, Owner memory+claim union, global/legacy
      fallback, NULL/exact-empty singleton and branch semantics have positive and
      cross-scope negative tests. The indexed route-ledger migration materializes
      creation plus every validated reroute, an A→B→C fixture discovers B without
      a creation/current B candidate, and Project/Owner before/equal/after uses
      route-at-t including `memory_type` and raw nullable `topic_key`. A real
      normal-save same-type raw-key transition has before/equal/after tests;
      type transition is not fabricated because all current save target selectors
      require the requested type. A stable-source-ID Markdown project→global
      fixture changes type/key and proves old Project membership/identity before,
      new Owner membership/identity at and after equality,
      `source_kind=markdown_import`, atomic rollback, and a
      missing-predecessor or legacy gap error. Backfill gaps, forward-only
      pre-floor reads and incomplete writer chains fail closed. All six current
      insert families, the three existing-row route writers, same-value no-ops,
      changed-route staging and direct bypass rejection are covered. A
      production-shaped FK fixture proves the `memories` rebuild first drops
      every external trigger that references it, recreates external triggers
      byte-for-byte, and defers every preexisting memory-owned UPDATE side-effect
      trigger—including FTS/enrichment—until replay exact-matches stored terminal bytes and dependent rows while preserving
      rows/DDL, restoring enforcement, and repeating `foreign_key_check`; the legacy user-claim wrapper stays
      user-claim-only, performs bounded referenced-memory plus applicable
      `user_claim`/`pattern` suppression reads, and is not failed by unrelated
      malformed exact-owner memory or memory-only suppression.
- [ ] Effective reference epoch, one-read-snapshot behavior, replayability enum,
      immutable duplicate-capture time and source/knowledge boundaries are
      golden-locked; current-only output is never labeled exact.
- [ ] Candidate-backed memory validates completion only against the initial
      route identity, then uses each cutoff state solely for membership and
      emitted `SubjectIdentity`; it never rematches candidate identity at cutoff.
      Missing/forked/forward-only chains and terminal drift fail closed; proved
      transitions retain the candidate trust cap. Workspace/user input scope maps
      through the writer route without requiring an unavailable candidate title.
      Operation-less procedure memory is current-snapshot-only; history excludes/Unknown.
- [ ] Versioned edit and in-place mutation histories have separate
      before/equal/after tests. One globally ordered lifecycle chain covers every
      production status writer, including save/Markdown, candidate/TTL/supersede,
      preference removal, and stale archive; gaps, forks, unsupported transitions and Web
      ledger mismatches return `unreconstructable_memory_lifecycle`. Its
      memory/time index is used, and deleting 30-day events leaves both ledgers,
      Web proof and serialized historical output unchanged. Every governed route/lifecycle writer's
      strict fingerprint/nonblank request-ID DDL, intent→trigger-v1→final-seal
      order, complete result mapping, unsealed-commit rejection, exact retry,
      different-payload conflict, same-second ordering, crash-before-commit and
      commit-success/response-loss/concurrent retry are covered without duplicate memories, mirrors or knowledge
      advancement. Save fixtures distinguish equal identity/different content;
      Markdown retry stays equal across importer metadata rewrite.
- [ ] Save requires a validated caller idempotency key. Same key/equal payload
      replays the byte-equivalent durable response; same key/different payload
      conflicts before mutation; different keys with an identical lesson payload
      execute twice and preserve reinforcement, operation and claim evidence.
      Raw keys/credentials never enter logs, fingerprints, response JSON or
      retained tables.
- [ ] The exact cutover DDL rejects UPDATE/DELETE on intent/result/seal rows,
      malformed or duplicate manifests, missing/extra/shape-invalid result
      bindings, orphan/mismatched INSERT origins, ledger appends without a typed
      manifest slot, ledger appends after seal, and seal when any ledger lacks
      its typed result or is nonterminal/current-row-mismatching. Owner DDL
      accepts exactly seven scopes and rejects blank
      or untrimmed keys. It
      rejects a write connection without the approved SHA-256 UDF. Golden frame
      vectors match Rust, trigger and migration backfill bytes; nonce/hash/digest
      columns reject BLOB, REAL, and INTEGER storage. Literal
      `memory_route_ledger_fingerprint_guard`,
      `memory_lifecycle_ledger_fingerprint_guard`, and
      `memory_insert_v1_ledgers`, `memory_route_tuple_update_guard`, plus
      `memory_write_commit_guard` SQL is
      normalized against `sqlite_schema`; all six current memory statuses,
      including `superseded`, are accepted while unknown values fail;
      every typed OLD/NEW field is hashed and memory+route-v1+lifecycle-v1 are
      one atomic statement outcome.
- [ ] The local-copy crash matrix covers journal reservation, staged-file fsync,
      durable prepublication new-pin intent/link/fsync, swap intent, backup
      hard-link pin, present-target atomic exchange, durable exchange/restore
      intents, restore pin, no-replace target evacuation/hold, and atomic
      N→G rollback quarantine and matching-seal B→O predecessor quarantine,
      each with both parent fsyncs before a final pin is removed. It also covers
      the durable `cleanup_intent` snapshot, every
      revalidation/ordered unlink prefix, absent-target no-replace publication,
      every database point through commit, cleanup and journal deletion.
      No-seal recovery restores prior bytes
      or absence when uncontested while retaining the displaced D1 indefinitely
      under nonce-qualified G; collision keeps latest bytes at target or under
      H/N/G with an explicit error. Sealed recovery keeps the new digest at
      target and any structurally proved, phase-drifted predecessor I0*
      indefinitely under O;
      tampering and indeterminate states stay visible. Temp naming/scanning/ownership
      and every legal target/backup/stage tuple have deterministic outcomes.
      Backup initially proves original identity/metadata/mode/digest, while each
      phase proves the exact known basename set and nlink for every pinned inode;
      normal rollback proves D0 `{target,B,S,C}` and D1 `{H,N}`, then atomically
      changes D1 to `{H,G}` and finally `{G}` without an unpinned window. A
      pre-boundary open-FD collision retains its inode and every recovery crash
      converges; after the cleanup boundary the target and nonpermanent pins
      remain quiescent while retained G/O may continue to receive old-FD drift.
      Portable absent publication includes the deterministic
      `{target,S,N}=D1`/nlink=3 link-before-unlink crash tuple. Prior-absent
      rollback is terminal only with target/H both absent and treats target≠G
      as collision; an existing H is classified before terminal. Only observed
      target=G is evacuated to H rather than pathname-unlinked. Exact H=G is
      removable, a raced-in H is restored no-replace, and EEXIST retains both
      entries as collision evidence.
      Cross-process faults at every writer phase prove scanner and doctor cannot
      recover a live request, including durable D1 before its database seal; after
      writer death exactly one anchor-verified lock owner reconciles. Lock-path
      replacement may acquire a second kernel lock but fails immutable
      fd/path/inode/nonce proof before any request or artifact access. Target-parent
      traversal, identity, ownership, permissions, device, fsync/no-replace support
      and every stage proof are fault-tested independently of the private journal
      parent. Renaming retained P and recreating its canonical path at inspection,
      read-lift begin/finish/restore/recovery, snapshot, cleanup-J load, and J unlink
      must return typed ambiguity without operating through either old or decoy P.
      First-use Q/locks and Q/quarantine creation, child and parent directory
      fsyncs, and the shared exact 32-lowercase-hex stage nonce grammar are
      crash- and boundary-tested.
      Every stage-build create/chunk-write/fdatasync/publish checkpoint converges;
      forged U/S proofs fail closed. Absent-target and
      link/exchange/evacuation source races
      prove a competing create/replace or open-FD write before
      `cleanup_intent` is never deleted, sealed, or misclassified; the durable
      hold either restores its observed entry no-replace or retains newer
      post-choice bytes under H/N/G/O visibly. A race between cleanup snapshot
      persistence and revalidation returns
      `local_copy_cleanup_concurrency_violation`; the harness keeps target
      quiescent after successful revalidation.
      The five exact cleanup sources and ordered lists reject every other
      source/list/seal tuple. Source J plus absent or exact-owned stale
      partial/complete `Tc`, one mode-qualified same-inode `V`, and combined
      `Tc`+`V` restart forms all have deterministic outcomes. Malformed
      group/mode suffixes and multiple marker candidates fail closed; only the
      fresh complete valid `Tc` may advance, and cleanup J rejects stray `Tc`.
      Restore fchmod/fsync faults retain V; retry uses its encoded original mode
      and fsyncs restoration before disarming it. Target replacement and open-FD content writes restore
      through a surviving alias, then return the typed concurrency error with
      `doctor_healthy=false`; journal identity ambiguity returns the distinct
      reconciliation error with the same nonhealthy state.
      Every cleanup name is removed only through no-replace source→Xc capture,
      Q/source-parent durability, retained-FD/Xc postproof and Xc unlink/Q-fsync.
      Proof→rename replacement and post-rename open-FD drift restore the exact
      captured bytes no-replace before error; EEXIST keeps Xc. All five capture
      and three restore crash boundaries restart by restoring Xc before prefix
      derivation. Multiple/malformed/unsafe Xc and V+Xc or Tc+Xc remain intact
      and ambiguous. All production safety invariants use explicit typed checks,
      never runtime assertions or hand-raised `AssertionError`; public journal,
      snapshot, inspection, transition, read-lift and cleanup boundaries map
      external proof failures to typed errors. Optimized execution independently
      preserves the V, Tc, and Xc gates. A second process stays lock-busy throughout active V, Tc, Xc,
      ordered cleanup and J removal; fork inheritance cannot reuse capability.
      Completed G/O files are reported separately from pending journals, have
      no automatic garbage collection, and a fresh attempt uses a distinct
      stage nonce; sealed exact replay remains mutation-free.
      Tests limit pre-boundary supported concurrency to the user target.
      Distinguishable reserved-name identity/type/ownership/link mutation is a
      security-visible ambiguity. Phase-qualified mode/content drift of a
      formerly exposed target inode under B/S/C/H/N/G/O is accepted through the
      cleanup boundary without claiming whether it came through an old target
      fd or reserved path.
- [ ] Canonical same-topic and cross-topic noops advance trust/ack knowledge only
      at their transition; malformed result provenance fails closed.
- [ ] Candidate replacement/no-op multi-active transitions reconstruct all
      validated co-predecessors before the boundary and fail closed on
      unexplained unlinked Superseded rows; duplicate-active ordering is fixed.
- [ ] User source kinds/ref shapes are total, and candidate wrapper equality,
      exact result/edit invariants, first-party explicit-user sources,
      host/project/session and binding/reference-time scope, exact per-kind ref
      counts, recursive application/edit time, duplicate and cycle handling
      fail closed. Every summary ref/status yields the documented Phase A
      provenance error without exposing content or trust; manual scalar path
      `0` and inherited provenance-root binding are golden-locked.
- [ ] Observation catalog shape/order/dedup/provenance and explicit attachment
      are covered, including NULL refs, read-time poisoning scan, external
      trust cap, empty-ref ModelGenerated default, nullable epoch errors and fact
      learned/created/valid/invalidation/replacement boundaries plus late-insert
      rejection; post-cutoff stale/compressed lifecycle mutation fails visibly
      and no implicit linkage exists.
- [ ] `poisoning_quarantined` and unknown Observation statuses cannot expose
      usable evidence.
- [ ] External/pack trust caps, WebFetch/MCP/network-Bash mixed evidence and
      legacy/default stored caps, unknown source class and SourceTrustClass
      no-self-uplift are covered, including later-epoch ID rejection and a
      network-fetching Bash command beyond the 16 KiB preview. Inline,
      current-hash blob and legacy-hash blob storage have positive fixtures at
      16,384/16,385-byte and multibyte boundaries, with the network marker only
      in the compacted-away middle.
- [ ] All seven canonical suppression targets, owner and active/revoked
      interval cases and exact match/shape matrix are covered without
      cross-owner hiding, including correctly typed non-applicable
      user-candidate/summary targets. Entity-link current-only replayability and
      explicit-history integrity errors prevent retroactive suppression.
- [ ] Same-identity decisions, cross-identity provenance, uniform preference
      conflict output/application boundaries and heterogeneous canonical pair
      behavior are covered, including overlapping-pair rejection and unbacked/
      candidate-only/operation-only provenance shapes. All six memory-edge kinds
      have exact direction/mapping tests and an unknown kind fails contextually.
- [ ] Malformed/dangling/foreign evidence and operation provenance fail closed
      with contextual diagnostics.
- [ ] SQLite authorizer/`total_changes` proves SELECT-only behavior while
      allowing bounded transaction control; a concurrent-writer fixture proves
      every stage observes one snapshot.
- [ ] Seed-933 chunk/high-fanout/unrelated-project validation passes and a
      final-head performance record is produced.
- [ ] README, architecture, changelog and distribution metadata document the
      v1→v2 migration and breaking release boundary.

## Later GH-933 Acceptance

- [ ] Phase B Context Bundle consumes one projection per render with one shared
      reference epoch, separate truth/decision/conflict output, error-visible
      failure and an old-path rollback.
- [ ] Worktree/task selectors have the same no-leak properties as project and
      branch scope.
- [ ] Archived evidence can support an explicitly designed historical
      explanation without entering current context by default.
- [ ] Phase C records a benchmark-backed decision for general Claim-writer
      convergence beyond Phase A's route/lifecycle history substrate;
      convergence, if chosen, has migration and rollback.
- [ ] Session-summary refs become usable only after an immutable complete
      generated-surface binding (or equivalent snapshot) is persisted.
- [ ] Absolute attachment ordering, if required, uses a durable sequence rather
      than second-resolution timestamp inference.
- [ ] Generated enrichment cannot create or overwrite a canonical Claim in the
      final read/write contract.

## Release Contract

The v1 public API is immutable release history. Because v2 changes legal-input
grouping, DTOs, selectors and output, it must ship in 0.7.0 or the
then-current next breaking SemVer boundary, never as a 0.6.x patch. Migration
docs use the real Rust path `remem::truth`.

Phase A v2 PRs use `Refs #933`. They do not close GH-933 or claim Phase B/C
delivery. Merge and release remain explicit human decisions.
The breaking migration is never reached through ordinary database open. A
single-use canonical plan binds database/binary/backup identity, and only the
operator's exact lowercase SHA-256 approval authorizes cutover. Plan checkpoints
WAL/closes handles before binding stable DB+backup hashes. Apply writes one
`approved` entry; preflight leaves it retryable, schema start marks it
`cutover_started`, and recovery resumes only the same exact attempt or verified target.
An `approved` attempt may be durably retired only while exact pre-cutover state is
unchanged; started attempts cannot retire. Plan journals preparation before its sole
backup so crash recovery adopts only exact output or cleans only exact owned temp.
Before consuming approval, the exact rebuild runs purely over every destination
type/domain/FK/API constraint and repeats under the write lock before durable
start. Windows plan/apply fail typed with zero side effects and retain v1 support.
