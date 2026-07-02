# Review Queue Throughput Product Spec

Status: Current contract
Date: 2026-07-02

Tracking:
- Epic issue: #683
- Related contracts: #674 (summary-candidate promotion policy),
  #588 (status/health performance)

## Problem

The promotion gate is intentionally strict, so most candidates land in
`pending_review`. That is the right safety posture, but the review side has
no throughput story:

- Review is per-id only: approve, discard, or edit one candidate at a time.
- No surface reports queue age, growth rate, or why candidates are blocked.
- When review stops, memory growth stops while capture keeps running. The
  user sees no warning; the flywheel silently stalls. Prior audits found
  block-reason classes that could never pass the gate — invisible until
  someone read the database.

A memory system whose curation queue can silently starve is not "the best
memory system" regardless of retrieval quality.

## Goals

- Queue health is observable: counts, ages, and block-reason distribution
  appear in `remem status --json` and as doctor findings with thresholds.
- Review effort scales: filter-scoped batch operations plus a fast
  sequential flow make clearing a hundred candidates a minutes-scale task.
- Systematic blockage becomes actionable: block-reason aggregates surface
  gate deadlocks as findings instead of stuck rows.

## Non-Goals

- No loosening or retuning of the auto-promote gate. Promotion policy for
  summary-derived candidates belongs to #674.
- No auto-approval heuristics. Every promotion remains gate-initiated or
  human-initiated.
- No web review UI in this cycle. The REST `candidates` endpoints stay the
  substrate for a later UI.
- No change to candidate extraction itself.

## Product Principles

### Stagnation Is A Health Failure

A growing, aging review queue is a system health signal equivalent to a
failing extraction worker. It must show up where users already look for
health: `remem status` and `remem doctor`, with concrete thresholds rather
than raw numbers only.

### Batch With A Preview, Never Blind

Batch operations always show what they are about to do (count, breakdown by
type and project) and require explicit confirmation. Filters select
candidates; humans confirm outcomes. A batch discard is destructive to
pending curation and must never run silently.

### Block Reasons Are For Fixing, Not Just Logging

`auto_promote_block_reason` exists today as a per-row diagnostic. Aggregated,
it answers "is the gate systematically rejecting a class it should not?" —
that aggregate is the primary tool for finding gate deadlocks.

## User Stories

### Visible Queue Health

As a user, `remem status` tells me how many candidates await review, how old
the queue is, and whether it is growing faster than I am reviewing.

Acceptance:

- `status --json` exposes pending count, median and max age, 7-day inflow vs
  resolved counts, split by project.
- `doctor` warns when median age or backlog growth crosses documented
  thresholds.

### Batch Review

As a user, I can review by slice instead of by row: approve all low-risk
discoveries for one project, or discard everything matching an obsolete
topic, after seeing a preview.

Acceptance:

- `remem review approve-batch` / `discard-batch` accept filters (project,
  type, block reason, minimum confidence, age) and print a preview requiring
  confirmation (`--yes` for automation).
- Batch outcomes are recorded like individual review outcomes, including
  who/what initiated them.

### Fast Sequential Review

As a user, I can walk the remaining judgment calls one keystroke per
candidate.

Acceptance:

- A sequential mode presents one candidate at a time with
  approve/discard/edit/skip actions and shows queue progress.

### Deadlock Surfacing

As a user, I learn from doctor — not from database spelunking — that a class
of candidates can never pass the gate.

Acceptance:

- Block-reason aggregate report exists (`remem review blocked` or
  equivalent) with counts and example rows per reason.
- Doctor emits a finding when one block reason dominates beyond a documented
  share and the affected class is gate-ineligible by construction.

## Rollout

1. Metrics: queue health in `status --json` + doctor thresholds.
2. Batch operations with preview/confirm, plus block-reason aggregate report.
3. Sequential review flow.

Each phase ships independently with focused tests plus:

```bash
cargo fmt --check
cargo check
cargo test
```

## Open Questions

- Where do thresholds live: hardcoded defaults with env overrides, or the
  config file health section from #588?
- Should batch approve respect a per-run cap to bound blast radius?
- Should review outcomes feed `memory_feedback` so future ranking work can
  learn from discard patterns (relates to #383)?
