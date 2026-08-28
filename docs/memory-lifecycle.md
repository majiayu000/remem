# Memory Lifecycle

remem memory is not append-only. Durable facts move through explicit lifecycle
operations so corrected facts can replace stale facts without losing audit
history.

## Operation Model

| Operation | When to use | Storage effect |
|---|---|---|
| `add` | New durable fact with enough evidence. | Insert an active memory. |
| `update` | A new fact corrects or replaces older facts. | Insert the replacement memory and mark superseded ids `stale`. |
| `invalidate` | Existing memories are known to be wrong or obsolete. | Mark the listed memories `stale`; do not delete their rows. |
| `noop` | Evidence is already represented or not durable. | Record an explicit no-op outcome; write no memory. |
| `defer` | Evidence is ambiguous, contradictory, or incomplete. | Leave durable memory unchanged and requeue/review the extraction task. |

`delete` is intentionally not a normal memory operation. Unless a future
privacy or retention feature requires hard deletion, invalidation should be a
soft state transition to preserve provenance and debugging history.

## State Semantics

`active` is a stored lifecycle state, not by itself proof that a memory is
current or eligible for every read surface. Current-memory classification also
checks provenance, confidence, validity, state identity, and quarantine state.
An active row that cannot prove those requirements is classified
`legacy_unverified`: explicit search and detail remain inspection/recovery
surfaces and may return it with a visible classification and reason, while
CurrentTruth and default SessionStart exclude it and audit the exclusion.

The [legacy-unverified visibility contract](specs/legacy-unverified-context/PRODUCT.md)
is authoritative for the ordered current-memory classes and the difference
between recovery reads and implicit context. `stale` rows remain historical
evidence and are excluded from default search unless a caller explicitly asks
to include them.

Current operational facts can also carry `expires_at_epoch`. Default retrieval
treats an expired active row as non-current even before cleanup runs. Cleanup
then marks expired active rows `stale` and keeps the row for provenance.

Default TTLs:

- local service/port/URL health and PR/CI/review state: 24 hours
- git branch divergence snapshots: 7 days
- product, architecture, verified bugfix, lesson, procedure, and user
  preference memories: no TTL unless explicitly superseded

## Automatic Maintenance

The worker probes SQLite for one database-global cleanup job. The durable
24-hour cooldown is measured from the latest completed automatic attempt;
process-local timers only reduce probe traffic. Cleanup has a dedicated claim
path ahead of extraction so a persistent extraction backlog cannot starve the
daily lifecycle convergence.

A real cleanup builds its plan and applies it in one immediate transaction.
The memory/workstream/event/observation effects, successful maintenance-run
record, and automatic job completion commit together. Any failure rolls back
all effects before a bounded failure record is appended and the job follows
normal retry handling.

Automatic hard deletion is fail-closed:

- only old legacy `events` rows explicitly classified `ephemeral` are eligible;
  governance/audit rows and any API-referenced event remain;
- a compressed source observation requires an old matching link, an active
  replacement, a complete supported snapshot/hash, and no remaining fact
  reference. New `observation-v2` links preserve all canonical content and
  provenance fields, revalidated from before the AI call through the atomic
  write. Mutable `status`/`last_accessed_epoch` and noncanonical joined
  `content_session_id` are not hashed; status is checked independently. An
  exact legacy v1 link is upgraded to v2 inside the deletion transaction when
  current canonical provenance was omitted; malformed/mismatched links and
  schemas with unknown observation columns are retained; and
- archived failures are never selected automatically. Their purge requires an
  explicit positive `remem cleanup --archived-failures[=DAYS]` horizon.

`remem doctor` reads the append-only automatic-run ledger rather than job
claim/retry timestamps. A newer failure is visible even when an older success
exists, and a later success restores healthy status without erasing history.

Candidate review state is separate from memory state:

| Candidate state | Meaning |
|---|---|
| `auto_promoted` | Low-risk candidate was promoted to an active memory. |
| `pending_review` | Candidate needs manual review before becoming durable memory. |
| `discarded` | Candidate was rejected after review. |
| `defer` outcome | No candidate row is created; the extraction task keeps the reason in `last_error` and retries later. |

Session summaries are not a shortcut to active memories. Durable decisions,
learned facts, and preferences derived from summaries must be written as
`pending_review` memory candidates with source event evidence, then promoted
through the same routing and lifecycle path as observation-derived candidates.

## Provenance Rules

Every promoted memory should keep enough provenance to explain why it exists:

- source session and project context
- evidence event ids
- source candidate id when promotion came from candidate extraction
- confidence for extracted candidates
- branch and file metadata when available

Superseded memories keep their original content and provenance. The replacement
memory points forward through normal fields such as `topic_key`, while the old
rows are preserved by `status='stale'`.

## Retrieval Rules

Default search excludes stale memories, so corrected facts outrank obsolete
facts by visibility rather than only by score tuning. As an explicit
inspection/recovery surface, search may still return active rows excluded from
CurrentTruth, including labeled `legacy_unverified` rows. Historical/debug
flows can set `include_stale=true` or query ids directly when they need to
inspect old facts.

The FTS/lifecycle boundary is a machine-checked contract:

<!-- remem-doc-contract:memories-fts-lifecycle:start -->
| Invariant | Value |
|---|---|
| Indexed statuses | active, stale, archived |
| Lifecycle visibility | post-JOIN query-time filter |
<!-- remem-doc-contract:memories-fts-lifecycle:end -->

Default search excludes stale and archived rows, while `include_stale=true`
enables historical reads. Index presence does not itself establish CurrentTruth
or default SessionStart eligibility; the current-memory classification applies
the additional proof and quarantine checks.

## Failure Handling

Ambiguous extraction is not the same as "no candidates." Extractors should use:

- `<no_candidates reason="..."/>` when the evidence is clear but not durable.
- `<defer reason="..."/>` when the evidence is ambiguous, contradictory, or not
  safe to decide automatically.

Worker handling maps `defer` to the extraction task retry/review path. This
keeps uncertain facts out of durable memory and avoids silent drops.

## Metrics To Track

The lifecycle should remain observable as more automation is added:

- write count by operation: add, update, invalidate, noop, defer
- stale/superseded count by project
- defer age and retry count
- candidate promotion rate versus pending review rate
- conflict/update tests in the evaluation suite
