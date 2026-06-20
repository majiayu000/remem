# User Context Layer Technical Spec

Status: Current contract
Date: 2026-06-20

Tracking:
- Spec: #574
- Manual claims and CLI governance: #575
- Editable profile summaries: #576
- Suppression and feedback controls: #577
- On-demand user recall: #578
- Guarded automatic extraction: #579

## Existing Implementation Facts

The design builds on current remem behavior rather than replacing it:

- Hooks write append-only evidence to `captured_events`, with large payloads in
  `event_blobs`.
- `extraction_tasks` coalesces background extraction work by
  host/project/session/task kind.
- Durable coding memory is created after extraction, candidate review, and
  promotion.
- Memory ownership fields already distinguish `source_project`,
  `target_project`, `owner_scope`, and `owner_key`.
- `memory_state_keys` already represent stable slots for mutable current facts.
- `context_injection_items` already records per-item context decisions.
- `memory_usage_events` and access counters already provide a usage feedback
  substrate.

## Design Rules

- User context claims are not ordinary coding memories.
- Profile summaries are compiled views, not canonical truth.
- Suppression is a policy layer, not deletion.
- Automatic extraction must create reviewable candidates before activation.
- No active user claim may lack source metadata.
- Fail closed on malformed extraction, schema errors, source loading errors, or
  summary refresh errors.
- Sensitive or speculative claims require explicit user approval.

## Schema

The manual-claim and summary phases use earlier user-context migrations.
Suppression and feedback controls are added by `v051_memory_suppressions_feedback`
without rebuilding existing core tables.

### `user_context_claims`

Stores current and historical user-context claims.

```sql
CREATE TABLE user_context_claims (
  id INTEGER PRIMARY KEY,
  user_key TEXT NOT NULL DEFAULT 'user:default',
  owner_scope TEXT NOT NULL,
  owner_key TEXT NOT NULL,
  claim_type TEXT NOT NULL,
  claim_key TEXT NOT NULL,
  claim_text TEXT NOT NULL,
  confidence REAL NOT NULL,
  sensitivity TEXT NOT NULL,
  source_kind TEXT NOT NULL,
  source_refs_json TEXT NOT NULL, -- non-empty JSON array
  status TEXT NOT NULL,
  valid_from_epoch INTEGER,
  valid_to_epoch INTEGER,
  last_confirmed_at_epoch INTEGER,
  supersedes_claim_id INTEGER,
  created_at_epoch INTEGER NOT NULL,
  updated_at_epoch INTEGER NOT NULL,
  FOREIGN KEY(supersedes_claim_id) REFERENCES user_context_claims(id)
);
```

Required indexes:

```sql
CREATE INDEX idx_user_context_claims_owner_active
  ON user_context_claims(owner_scope, owner_key, claim_type, claim_key, status);

CREATE INDEX idx_user_context_claims_user_recent
  ON user_context_claims(user_key, updated_at_epoch DESC);

CREATE INDEX idx_user_context_claims_status
  ON user_context_claims(status, updated_at_epoch DESC);
```

Allowed statuses:

```text
active | pending_review | stale | superseded | suppressed | rejected | deleted
```

Allowed sensitivity values:

```text
normal | personal | sensitive | restricted
```

`deleted` is an audit status for soft deletion. A future hard-delete command may
physically remove rows only behind explicit confirmation.

### `user_context_candidates`

Added in the automatic extraction phase. Manual `remember` can write claims
directly and does not need this table.

```sql
CREATE TABLE user_context_candidates (
  id INTEGER PRIMARY KEY,
  user_key TEXT NOT NULL DEFAULT 'user:default',
  owner_scope TEXT NOT NULL,
  owner_key TEXT NOT NULL,
  source_project TEXT,
  host TEXT,
  session_id TEXT,
  claim_type TEXT NOT NULL,
  claim_key TEXT,
  claim_text TEXT NOT NULL,
  confidence REAL NOT NULL,
  sensitivity TEXT NOT NULL,
  risk_class TEXT NOT NULL,
  source_kind TEXT NOT NULL,
  source_refs_json TEXT NOT NULL,
  source_preview TEXT,
  review_status TEXT NOT NULL,
  auto_promote_block_reason TEXT,
  review_note TEXT,
  result_claim_id INTEGER,
  created_at_epoch INTEGER NOT NULL,
  updated_at_epoch INTEGER NOT NULL,
  FOREIGN KEY(result_claim_id) REFERENCES user_context_claims(id)
);
```

Allowed review statuses:

```text
pending_review | auto_promoted | approved | edited | rejected | suppressed | deferred
```

### `user_context_summaries`

Stores compiled user/profile summaries.

```sql
CREATE TABLE user_context_summaries (
  id INTEGER PRIMARY KEY,
  user_key TEXT NOT NULL DEFAULT 'user:default',
  owner_scope TEXT NOT NULL,
  owner_key TEXT NOT NULL,
  scope TEXT NOT NULL,
  scope_key TEXT,
  summary_text TEXT NOT NULL,
  source_claim_ids_json TEXT NOT NULL,
  source_memory_ids_json TEXT NOT NULL,
  source_activity_refs_json TEXT NOT NULL,
  status TEXT NOT NULL,
  model TEXT,
  version INTEGER NOT NULL,
  created_at_epoch INTEGER NOT NULL,
  updated_at_epoch INTEGER NOT NULL
);
```

Summaries should be replaced by inserting a new version or updating one current
row with enough operation logging to preserve history. Choose the simpler path
in the implementation PR, but tests must prove the previous summary is not lost
when refresh fails.

### `memory_suppressions`

Applies "do not mention/use by default" policy to claims, memories, topics,
entities, patterns, or summary lines.

```sql
CREATE TABLE memory_suppressions (
  id INTEGER PRIMARY KEY,
  owner_scope TEXT,
  owner_key TEXT,
  target_kind TEXT NOT NULL,
  target_id INTEGER,
  target_value TEXT,
  reason TEXT NOT NULL,
  actor TEXT NOT NULL,
  status TEXT NOT NULL,
  created_at_epoch INTEGER NOT NULL,
  updated_at_epoch INTEGER NOT NULL
);
```

At least one of `target_id` or `target_value` must be present. Valid target
kinds include:

```text
memory | user_claim | user_candidate | topic_key | entity | pattern | summary
```

### `memory_feedback`

Records relevance and quality feedback without mutating the target row.

```sql
CREATE TABLE memory_feedback (
  id INTEGER PRIMARY KEY,
  target_kind TEXT NOT NULL,
  target_id INTEGER,
  target_value TEXT,
  feedback TEXT NOT NULL,
  source TEXT NOT NULL,
  context_injection_item_id INTEGER,
  session_id TEXT,
  project TEXT,
  reason TEXT,
  created_at_epoch INTEGER NOT NULL,
  FOREIGN KEY(context_injection_item_id) REFERENCES context_injection_items(id)
);
```

At least one of `target_id` or `target_value` must be present. Use
`target_value` for text-keyed targets such as `topic_key`, `entity`, and
`pattern`; do not overload unrelated integer ids.

Allowed feedback values:

```text
relevant | not_relevant | harmful | stale | too_noisy
```

Ranking changes must remain behind configuration until tested against
retrieval regressions.

Default retrieval, context, preference, lesson, current-state, summary-source,
REST search/browse/graph/detail, and MCP search/detail paths must apply active
suppressions unless the caller explicitly sets an audit-only
`include_suppressed` flag.
Feedback events are recorded for later analysis and must not alter ranking by
default.

## Activity Timeline

Do not add a `user_context_activities` table in the first slice unless the
implementation needs it. The initial activity timeline should be computed from
existing durable sources:

- `session_summaries`
- `workstreams`
- repo memories with `memory_type=session_activity`
- commit/session links where available

If this becomes too slow or lossy, add a later table with source refs and
importance scoring.

## CLI Surface

Add a top-level `user` command group:

```text
remem user remember [--scope user|workspace|repo|session] [--owner-key <key>] <text>
remem user claims list
remem user claims why <id>
remem user claims edit <id>
remem user claims suppress <id>
remem user claims unsuppress <id>
remem user claims delete <id>
remem user summary show
remem user summary refresh
remem user summary edit
remem user summary sources
remem user recall <query>
remem user review inbox
remem user review approve <id>
remem user review edit <id>
remem user review reject <id>
remem user review suppress <id>
```

Manual claims default to `--scope user --owner-key user:default`. Narrower
workspace, repo, or session ownership must be explicit; CLI validation must reject
missing or malformed owner keys for non-user scopes. Add stable `--json` output
for list/show/summary/recall/review commands before MCP relies on them.

## MCP Surface

Implemented in the #578 slice:

```text
recall_user_context
```

The MCP tool shares behavior with:

```text
remem user recall <query>
POST /api/v1/user/recall
```

Later user-context tools remain future work:

```text
get_user_profile
search_user_claims
search_user_timeline
review_user_context_candidates
suppress_user_context
unsuppress_user_context
explain_user_context
```

Each MCP result must include stable ids, source refs, and reason/drop codes.
Empty results should be explicit and non-error unless input is invalid.

## Context Integration

SessionStart remains compact:

1. Load active, non-sensitive, non-suppressed user preferences relevant to the
   current host/repo.
2. Optionally include a short profile summary if configured and relevant.
3. Exclude restricted/sensitive/personal claims by default unless explicitly
   approved for startup use.
4. Exclude suppressed/rejected/deleted/expired claims.
5. Record any injected user-context item in `context_injection_items` or a
   compatible audit surface.

Longer user context must be retrieved through `recall_user_context`.

## Recall Algorithm

`recall_user_context` should:

1. Normalize project/cwd, host, task intent, query, and owner filters.
2. Load candidate profile summaries for `user:user:default`, current
   workspace, and current repo.
3. Load active claims matching query/task/owner, excluding unsafe statuses and
   sensitivity classes.
4. Load repo memories, current-state answers, workstreams, and recent sessions
   relevant to the task.
5. Apply suppressions and expiry filters.
6. Rank with existing retrieval signals first; add feedback/usage ranking only
   when configured.
7. Return compact, source-attributed context with included and dropped reason
   codes.

The initial implementation resolves current-state only for explicit
`state_keys` supplied by the caller; it does not guess arbitrary state keys from
free-form task text.

No step may invent profile data. No-data returns an explicit empty result.

## Candidate Review Lifecycle

Implemented in the first #579 slice:

1. `v052_user_context_candidates` adds review-gated candidate rows with owner,
   source, risk, sensitivity, preview, status, block reason, review note, and
   result claim id.
2. `remem user review inbox` lists pending candidates.
3. `approve` and `edit` apply a candidate to `user_context_claims` only after a
   stable `claim_key` is present. Pending candidates may omit a key, but
   activation must fail closed until the extractor supplies one or a reviewer
   edits one in.
4. Matching active claims noop on identical text/sensitivity or become
   superseded before a replacement active claim is inserted.
5. `reject` and `suppress` close candidates without creating active claims.
6. Low-risk explicit user statements may auto-promote only when source refs are
   non-empty, a stable `claim_key` is present, sensitivity is `normal`, risk is
   `low`, confidence is at least `0.9`, and source kind is
   `explicit_user_statement`; all other candidates keep a block reason and stay
   pending review.

## Automatic Extraction

Implemented in source version `0.5.116`. Session rollup completion enqueues a
`user_context_candidate` extraction task for the same captured-event range. The
extractor loads raw captured events plus the matching session summary, builds a
strict JSON prompt, validates the full model response before any write, and then
persists only through `user_context_candidates`.

The prompt/parser contract requires:

- claim type
- claim key
- claim text
- confidence
- sensitivity
- risk class
- source kind
- non-empty `source_event_ids` that cite loaded captured events

Malformed output fails closed and creates no candidate or active claim.
Sensitive, speculative, assistant-authored, non-user-authored, medium/high risk,
low-confidence, and non-preference/non-constraint candidates stay pending review
with a block reason. Auto-promotion requires a normal, low-risk, high-confidence
explicit user statement, a stable `claim_key`, and cited source events that are
actually user-authored and support the claim text. Contradictory candidates
update/supersede/noop through the candidate lifecycle rather than append
conflicting active rows.

## Tests

Minimum test coverage by phase:

- migration/schema drift tests for every new table and index;
- store tests for active/default filters and status transitions;
- CLI parse/action tests for every command;
- summary compiler tests for source filtering and refresh failure behavior;
- suppression tests proving default search/context/recall exclusion;
- feedback tests proving events are recorded without unexpected ranking changes;
- MCP tool schema and behavior tests for recall/review tools;
- parser tests for automatic extraction malformed/sensitive/speculative cases;
- context tests proving compact startup and no sensitive/suppressed leakage.

For implementation PRs, run focused tests first, then:

```bash
cargo fmt --check
cargo check
```

Run `cargo test` before submission when touching shared runtime behavior,
schema migrations, extraction, MCP, or context injection.

## Rollout Plan

1. #575: manual claims and CLI governance.
2. #576: editable summaries.
3. #577: suppression and feedback controls.
4. #578: on-demand recall CLI/MCP.
5. #579: guarded automatic extraction and review inbox.

Each phase should ship independently with tests and documentation updates. Do
not widen auto-promotion beyond explicit low-risk user-authored preference and
constraint statements without adding review-gated tests first.
