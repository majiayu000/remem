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

`active` memories are the current facts used by default retrieval. `stale`
memories remain in the database as historical evidence but should not rank as
current facts unless a caller explicitly asks to include stale entries.

Candidate review state is separate from memory state:

| Candidate state | Meaning |
|---|---|
| `auto_promoted` | Low-risk candidate was promoted to an active memory. |
| `pending_review` | Candidate needs manual review before becoming durable memory. |
| `discarded` | Candidate was rejected after review. |
| `defer` outcome | No candidate row is created; the extraction task keeps the reason in `last_error` and retries later. |

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
facts by visibility rather than only by score tuning. Historical/debug flows can
set `include_stale=true` or query ids directly when they need to inspect the old
facts.

FTS maintenance follows the same rule: active memories are searchable as current
facts, while stale rows are removed from the active FTS index by the existing
status triggers.

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
