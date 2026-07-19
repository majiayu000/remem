# Tech Spec

## Linked Issue

GH-854

## Product Spec

[`product.md`](product.md)

## Codebase Context

| Area | Files | Current behavior | Change |
| --- | --- | --- | --- |
| Context policy | `src/context/policy.rs` | Existing item/character limits; no governed global k | Add `sessionstart_relevance_k`, defaulted from evidence and overridable by env |
| Query and candidates | `src/context/implicit_query.rs`, `src/context/query.rs`, `src/context/types.rs` | Hybrid memory query plus separately ordered lessons/sessions | Retain a bounded candidate pool and the implicit query for one selection plan |
| Relevance | `src/context/prompt_submit.rs` | Significant-token overlap is local and PromptSubmit-only | Extract the tokenizer/overlap scorer for reuse without changing PromptSubmit behavior |
| SessionStart render | `src/context/render.rs`, `src/context/sections/*` | Core and section budgets render independently | Freeze existing Core output, select governed candidates, then apply existing section budgets |
| Audit | `src/context/audit.rs` | Memory/workstream injected and dropped rows; score column already exists | Record governed scores and closed drop reasons; add session-summary item identity using existing summary IDs |
| Status | `src/db/query/status_spend.rs`, `src/cli/actions/query/status*` | Latest chars/runs and AI spend only | Aggregate latest available relevance policy/audit row into text/JSON |
| Eval | `src/eval/golden/*`, `src/eval/injection/*`, `src/eval.rs` | Existing k-aware golden, capacity, and injection reports | Reuse them for four arms and commit one decision report |

## Design

### Relevance score

Move the existing significant-token normalization and stop-token rules into a
private context relevance module. PromptSubmit continues to pass when at least
one significant token overlaps.

For SessionStart, tokenize the implicit query once. For each governed candidate,
count distinct query tokens contained in its normalized title/body text:

```text
score = matched_significant_query_tokens / significant_query_tokens
```

Candidate text is title+body for memory/lesson items and request+completed for a
session summary. Scores are local, deterministic, finite, and require no I/O.
No query or candidate text is added to logs, status, or eval metadata.

### Selection

1. Render Core into a temporary buffer with the existing selector and limits.
   This freezes the exact Core IDs and bytes without changing Core code.
2. Build governed candidates from Lessons, non-Core MemoryIndex items excluding
   frozen Core IDs, and Sessions.
3. When k is zero, bypass relevance and keep the legacy governed vectors.
4. Otherwise score candidates and discard zero-score items.
5. Sort by score descending, section order
   `Lessons < MemoryIndex < Sessions`, then stable identity.
6. Derive the threshold:
   - at least k positive candidates and a lower `(k+1)` score: midpoint of kth
     and next lower score;
   - kth score tied or no lower positive score: kth score;
   - fewer than k positive candidates: lowest positive score;
   - no positive candidates: no threshold and blank selection.
7. Keep at most k candidates; candidates at/above the threshold but outside k
   receive `sessionstart_k_limit`.
8. Feed the selected vectors to the existing section renderers and then the
   existing total/gate truncation path.

Stable identity is `memory:<id>` for Lessons/MemoryIndex and
`session_summary:<id>` for Sessions. Querying session summaries includes the
existing `session_summaries.id`; no migration is required.

### Audit and status

The selection plan supplies each governed item's score and pre-render
disposition to `build_context_audit_items`. Rendered survivors are `injected`;
zero-score items use `below_sessionstart_relevance_threshold`; eligible items
outside k use `sessionstart_k_limit`; selected items omitted by a section keep
`section_budget`. Existing final gate/truncation finalization remains the source
of final injected truth.

Each run also writes one `item_kind=sessionstart_relevance_policy` audit row.
Its title appears in the footer, its score stores the derived threshold, and its
provenance is a closed key/value string containing policy version, k,
candidate/eligible/selected counts, and state. It contains no query or memory
text.

`query_latest_session_memory_spend` treats `context_injection_items` as optional
for legacy compatibility. For the latest item run associated with the latest
session, it returns policy state, k, threshold, final governed injected count,
and grouped closed drop counts. Missing policy rows yield `unavailable`.

### Evaluation and recommendation

Run the existing golden evaluator at k 1, 3, 5, and 10 against
`eval/golden.json`. For every populated slice, compute the best `hit_at_k`;
an arm is eligible when its slice value is at least `best - 0.01` for every
populated slice. Choose the smallest eligible k.

For each arm, run the existing eval-gates path with
`REMEM_CONTEXT_RELEVANCE_K=<k>` and record:

- `source_reports.injection.metadata.output_chars`;
- `source_reports.capacity.degradation`;
- gate pass/fail and source artifact hash.

The committed report records source revision, dataset hash, commands, all
primary/secondary metrics, and the decision. It is generated from existing
evaluators and does not create a second gate.

## Product-to-Test Mapping

| Invariant | Area | Verification |
| --- | --- | --- |
| P-001, P-002 | context render | Off/legacy snapshot plus Core/Preferences/Workstreams non-regression tests |
| P-003–P-007 | relevance selector/policy | Unit tests for zero overlap, k, ties, sparse/no-positive scores, env default/override |
| P-008 | footer/stats | Render snapshot distinguishes disabled/applied/blank and drop counts |
| P-009 | status query/CLI | Legacy-unavailable and latest-policy text/JSON tests |
| P-010, P-011 | eval report | Four-arm completeness, all-slice eligibility, tie and missing-arm tests |
| P-012 | audit | DB assertions for score and each closed drop reason |

## Data Flow

```text
existing SessionStart inputs
  -> existing implicit query and bounded candidates
  -> existing Core selector (temporary buffer; frozen IDs/bytes)
  -> shared local significant-token scores for governed candidates
  -> threshold + global k
  -> existing section budgets
  -> existing total/gate path
  -> footer + context_injection_items
  -> latest-session status aggregation
```

## Alternatives

- Per-section k was rejected because the approved golden k is one retrieval
  result count and would permit up to three times the measured default.
- A new model/reranker was rejected because the maintainer required reuse and
  offline bounded execution.
- Coding-bench and signed-tag machinery were removed because they contradicted
  the approved 2026-07-19 charter.

## Risks

- Security: no new external calls or persisted query text; status contains only
  counts, score, version, and k.
- Compatibility: the default is intentionally tighter; k=0 restores legacy
  governed selection.
- Performance: scoring is linear in the bounded candidate pool and token set.
- Maintenance: one shared private tokenizer prevents PromptSubmit and
  SessionStart relevance rules from drifting.

## Test Plan

- [ ] Focused context relevance, render, audit, and status tests.
- [ ] `cargo fmt --check`
- [ ] `cargo check`
- [ ] `cargo clippy --all-targets -- -D warnings`
- [ ] `cargo test`
- [ ] `cargo run -- eval-gates --json-out /tmp/remem-eval-gates.json`
- [ ] Plugin/version synchronization and PR preflight.

## Rollback

Set `REMEM_CONTEXT_RELEVANCE_K=0` to restore legacy governed-section selection.
Reverting the implementation commit removes the new policy and additive status
fields without a data migration.
