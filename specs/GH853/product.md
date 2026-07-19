# Product Spec

## Linked Issue

GH-853

complexity: large

## User Problem

remem persists typed, provenance-bearing `graph_edges`, but standard memory
search does not consume them. Associative queries can therefore miss a memory
that is connected to a lexical or vector seed through a trusted entity, file,
or lifecycle edge even when the database already contains that relationship.

The accepted graph ADR keeps production traversal frozen until a literal
`graph_edges` arm demonstrates measurable associative benefit without
regressing ordinary retrieval. GH-853 must supply that evidence and may wire
the channel only when the same-head gate passes.

## Goals

- Add bounded, deterministic, SQLite-only traversal from eligible FTS/vector
  memory seeds over trusted `graph_edges`.
- Rank one-edge `supersedes` targets and two-edge `mentions` / `touches_file`
  memory co-neighbors as an independent `graph_traversal` RRF channel.
- Evaluate the literal channel on the pre-registered associative slice before
  it becomes a production default.
- Preserve project, branch, memory type, active/current, expiry, staleness, and
  suppression behavior of standard search.
- Expose graph channel timing and stable empty/disabled reasons in search
  explain output.

## Non-Goals

- Personalized PageRank, random walks, graph databases, network services, or
  schema migrations.
- SessionStart/context fusion changes.
- Ranking `diagnostic_hint` edges.
- Returning `extracted_from` episode nodes as memories; within the current
  two-edge endpoint contract they are diagnostic only.
- Inventing missing trusted edges or changing graph writer behavior.

## Behavior Invariants

1. `B-001`: Graph expansion starts only from memory IDs already returned by
   eligible, suppression-filtered FTS or vector channels for the same query.
2. `B-002`: Only currently valid `edge_trust=trusted` edges participate.
   `diagnostic_hint` edges never add a candidate or change rank.
3. `B-003`: Rankable paths are limited to one-edge memory-to-memory
   `supersedes` and two-edge memory-to-entity/file-to-memory co-neighbor paths
   using `mentions` or `touches_file`; traversal never exceeds two edges.
4. `B-004`: Every returned target satisfies the original search project,
   branch, memory type, active/current, expiry, staleness, and suppression
   policy. Shared entity/file nodes cannot reveal cross-scope IDs or payloads.
5. `B-005`: Traversal is bounded by positive seed, edge-scan, per-node degree,
   and candidate limits. Cycle and diamond paths are deduplicated, with stable
   ordering for identical input and database state.
6. `B-006`: A missing or empty `graph_edges` table, no eligible seed, and no
   eligible expansion are distinguishable successful empty outcomes. SQL,
   decode, or contract errors propagate and are not reported as empty graph.
7. `B-007`: Standard search explain reports a `graph_traversal` channel,
   including a per-channel timing and a stable `disabled_reason` when it has no
   usable result.
8. `B-008`: The literal A/B gate uses the same dataset and search code tree for
   both arms. Its primary slice is `associative`; it requires evidence
   recall@5 delta at least `0.05` and at least one real two-edge evidence hit.
9. `B-009`: Every non-associative slice is a non-regression gate for hit,
   recall, evidence recall, nDCG, abstention, and project leakage; p95 literal
   traversal latency must remain at or below `1000ms`.
10. `B-010`: Default production graph weight is non-zero only when the current
    head's committed graph-decision report passes all gates. A failing or
    incomplete report keeps the production channel disabled.
11. `B-011`: Traversal uses parameterized SQLite queries, performs no writes,
    makes no network or LLM calls, and does not expose graph node payloads or
    provenance contents in diagnostics.
12. `B-012`: `extracted_from` observations are diagnostic counts only in this
    tranche because the typed endpoints cannot produce a memory target within
    two edges. PPR is not a hidden fallback.

## Acceptance Criteria

- [ ] Literal `graph_edges` traversal passes focused trust, validity, path,
      cap, deterministic-order, dedupe, and scope-filter tests.
- [ ] The committed graph decision evaluates literal traversal on all 15
      associative fixtures and records delta `>= 0.05` plus a genuine two-edge
      evidence hit.
- [ ] Non-associative quality, abstention, and scope-leak checks do not regress,
      and p95 graph latency stays within `1000ms`.
- [ ] Default search exposes a timed `graph_traversal` RRF channel with safe
      empty reasons and no `diagnostic_hint` contribution.
- [ ] No graph database, migration, context wiring, or PPR implementation is
      introduced.
- [ ] Focused tests, full Rust checks, deterministic eval gates, version-sync,
      SpecRail workflow checks, and PR preflight pass.

## Edge Cases

- Empty graph / no seed / no expansion: successful empty channel with distinct
  explain reason.
- Expired edge: ignored at the reference time boundary.
- Cycle or diamond: candidate appears once at its best deterministic path.
- Cross-project, wrong branch/type, stale, expired, suppressed, or obsolete
  target: filtered before channel output.
- Malformed row or SQL failure: returned as an error, never silently disabled.
- Seed also reached through graph: seed is excluded from graph results.

## Release Notes

This is an additive retrieval channel over the existing SQLite graph contract.
No migration or external service is required. Rollback is a graph weight of
zero or reverting the channel wiring; existing graph data remains valid.
