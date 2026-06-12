# ADR: Freeze `graph_edges` as a Retrieval Channel Until It Clears the Graph Decision Gate

Date: 2026-06-12

## Status

Accepted.

## Decision

Do not wire first-class `graph_edges` traversal into the default retrieval stack yet.

Keep the existing explicit `--multi-hop` entity expansion path available, but treat
`graph_edges` as a provenance/candidate contract until a future pre-registered
A/B eval shows a material retrieval gain.

## Drivers

- Issue #382 requires a wire-or-freeze decision for graph retrieval before
  unblocking the older graph roadmap issues.
- The current deterministic golden corpus already has 50 fixture-backed queries,
  including 10 `multi_hop` cases.
- The current `multi_hop` slice is saturated under standard retrieval:
  `recall_at_k = 1.0` and `evidence_recall_at_k = 1.0`.
- Adding graph traversal to retrieval creates privacy/filtering and ranking risk,
  so it needs evidence before becoming a search channel.

## Evidence

Committed artifact: `eval/graph-decision/report.json`.

Command:

```bash
remem eval-graph-decision --json-out eval/graph-decision/report.json
```

Summary from the committed report:

- Decision: `freeze_graph_edges_retrieval_channel`
- Benefit threshold: `0.05`
- Multi-hop evidence recall delta: `0.0`
- Multi-hop recall delta: `0.0`
- Multi-hop nDCG@10 delta: `0.0`
- Non-multi-hop evidence recall delta: `0.2666666666666667`
- Entity-BFS p95 latency: `13.938834ms`
- Checks: `non_multi_hop_zero_regression = true`,
  `p95_latency_within_budget = true`, `all_checks_passed = true`

## Consequences

- `graph_edges` remains a typed graph/provenance contract, not a retrieval
  channel.
- #332, #334, and #335 should remain frozen unless a future eval expands the
  graph fixture with harder cases and shows a pre-registered gain.
- Future graph retrieval work must come with an A/B artifact, filter-leakage
  tests, and explainable path output before it can change defaults.

## Follow-ups

- Expand the multi-hop fixture with cases where standard retrieval does not
  already saturate recall.
- If a future graph channel is attempted, gate it behind explicit configuration
  first and preserve project, branch, owner, memory type, and stale filters.
- Re-run `remem eval-graph-decision` after fixture expansion or ranking changes
  before reopening graph retrieval implementation work.
