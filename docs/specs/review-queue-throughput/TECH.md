# Review Queue Throughput Technical Spec

Status: Current contract
Date: 2026-07-02

Tracking:
- Epic issue: #683
- Related contracts: #674, #588

## Existing Implementation Facts

- `memory_candidates` carries `review_status`
  (`pending_review` / `auto_promoted` / discard states), `confidence`,
  `risk_class`, `scope`, `memory_type`, `topic_key`, `evidence_event_ids`,
  and `auto_promote_block_reason`.
- The gate lives in `should_auto_promote` (`src/memory_candidate.rs`):
  project scope + low risk + confidence threshold + repo-owned route +
  routing confidence + evidence ids + auto-promotable type + no unsafe
  marker + source-observation support.
- CLI review surface (`src/cli/types.rs`): `ReviewAction::{List, Approve,
  Discard, Edit}` — all per-id except `List` (default limit 20). Graph
  candidates have a parallel `GraphReviewAction` with Inspect/Defer.
- REST already exposes `candidates` list/review/edit endpoints
  (`src/api/`).
- `remem status --json` and `doctor` exist with an established finding
  model (`src/doctor/`), including capture-liveness checks.
- Review outcomes currently update candidate rows; promotion writes into
  `memories` with provenance (`source_candidate_id`).

## Design Rules

- Read paths for metrics must be cheap: single aggregate queries over
  indexed columns; no per-row scans inside `status` (respect the #588
  fast-liveness vs cached-aggregate split).
- Batch mutations run in one transaction per confirmed batch and record the
  same per-candidate outcome data as individual actions.
- Preview and mutation must use the same filter resolution code path so the
  preview cannot diverge from what executes.
- Destructive batch actions (`discard-batch`) require explicit confirmation;
  `--yes` exists for scripts but is never the default.
- No gate changes: this spec adds observation and throughput around
  `should_auto_promote`, never edits its predicate.

## Phase 1: Queue Health Metrics

### Queries

Add aggregate readers (new module `src/memory/review_stats.rs` or extension
of existing status queries):

- pending count total and per project;
- median and max `created_at` age of `pending_review` rows;
- inflow (candidates created last 7d) vs resolved (approved+discarded last
  7d);
- count per `auto_promote_block_reason`.

Indexes: verify `memory_candidates(review_status, created_at)` is covered;
add a migration only if the query plan shows a scan.

### Surfaces

- `status --json`: new `review_queue` object with the fields above.
- `doctor`: findings with default thresholds — warn when median age exceeds
  14 days, or when 7-day inflow exceeds resolved by more than 3x with a
  backlog above 50. Thresholds documented in the doctor output and
  overridable via env until #588 decides a config home.

### Tests

- Seeded-fixture tests for each aggregate.
- Doctor threshold boundary tests (just below / just above).

## Phase 2: Batch Operations + Block-Reason Report

### CLI

```text
remem review approve-batch [--project P] [--type T] [--block-reason R]
                           [--min-confidence C] [--older-than DAYS]
                           [--limit N] [--yes]
remem review discard-batch  (same filters) [--reason TEXT] [--yes]
remem review blocked        [--project P]   # aggregate report
```

Behavior:

- Both batch commands resolve filters to a candidate id set, print a preview
  (count, per-type and per-project breakdown, first K sample rows), then
  prompt unless `--yes`.
- Approval reuses the existing single-candidate promotion path per id inside
  one transaction, so provenance (`source_candidate_id`, evidence ids)
  stays identical to individual approval.
- `--limit` defaults to a documented cap (e.g. 200) to bound blast radius;
  exceeding it requires an explicit higher `--limit`.
- `blocked` prints counts per `auto_promote_block_reason` with up to 3
  example candidate ids each.

### Doctor integration

- Finding when a single block reason exceeds a documented share (e.g. >60%
  of pending) and its class is structurally gate-ineligible — the deadlock
  signal.

### Tests

- Filter-resolution parity test: preview set == mutated set.
- Transactionality test: induced failure mid-batch leaves no partial state.
- Cap test: default limit enforced, override honored.

## Phase 3: Sequential Review Flow

- `remem review next [--project P]`: loop that renders one candidate
  (text, type, topic key, confidence, block reason, evidence summary) and
  accepts single-key approve / discard / edit / skip / quit.
- Implemented over the existing per-id actions; no new mutation paths.
- Non-interactive environments (no TTY) get a clear error pointing to batch
  commands.

### Tests

- Action dispatch tests with a scripted input stream.
- TTY-absence behavior test.

## REST Parity

Phase 2 filters and the blocked aggregate are added to the existing
`candidates` REST endpoints so a later UI consumes the same contract. No new
auth surface.

## Verification

```bash
cargo fmt --check
cargo check
cargo test
```

Manual smoke: seed candidates via a captured session, run
`remem status --json | jq .review_queue`, `remem review blocked`, one
batch preview, one sequential pass.

## Open Questions

- Config home for thresholds (pending #588 outcome).
- Whether review outcomes should also write `memory_feedback` rows now or
  wait for the #383 usage-loop contract.
- Whether `GraphReviewAction` gets the same batch treatment in this cycle or
  a follow-up.
