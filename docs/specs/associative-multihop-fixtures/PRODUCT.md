# Associative Multi-Hop Fixtures Product Spec

Status: Current contract
Date: 2026-07-02

Tracking:
- Spec/tracking issue: #676
- Related: docs/adr/2026-06-12-graph-decision-gate.md, frozen issues #332/#334/#335

## Problem

The graph decision gate froze `graph_edges` literal traversal because
multi-hop delta was 0.0 — but the fixture it was measured on is saturated:
the existing multi_hop golden queries are answerable by single-channel
retrieval, so no graph mechanism could have shown a delta on them. The freeze
was the right call on that evidence, but the evidence cannot distinguish
"graph adds nothing" from "the fixture cannot detect what graph adds".

External measurement (HippoRAG-2, ICML 2025) shows structural retrieval wins
only on associative queries — where the answer memory shares no lexical or
semantic overlap with the query and is reachable only through an intermediate
entity — and is at parity elsewhere. remem's golden set contains no such
queries, so the unfreeze preconditions in the ADR ("expand multi-hop fixture",
"literal traversal eval arm") cannot currently produce a meaningful verdict.

## Goals

- Give the graph gate discriminative evidence: golden queries that baseline
  channels genuinely fail and entity-hop retrieval could plausibly answer.
- Run both eval arms named by the ADR (entity-BFS proxy and literal
  `graph_edges` traversal) against the new slice.
- Re-record the gate decision with the new delta, whichever way it goes.

## Non-Goals

- Unfreezing or wiring graph traversal into production retrieval — evidence
  first, per the ADR; any unfreeze is its own decision after this spec's
  evidence lands.
- New graph tables, schema, or write paths.
- General fixture growth unrelated to associativity (capacity growth is
  #675).

## Behavior Invariants

1. At least 15 new golden queries are associative by construction: the
   judged-relevant memory shares an entity (file path, crate name, error
   signature, issue number) with an intermediate memory, while the query
   itself has near-zero lexical overlap with the judged memory's content.
2. Discriminative power is verified, not assumed: the baseline (current
   default channel set) is run on the new slice and its R@5 is recorded;
   if the baseline already saturates the slice, the fixtures are reworked
   before any gate re-evaluation.
3. Each fixture documents its intended hop path (query -> entity -> target)
   so failures are diagnosable.
4. Both eval arms run against the same slice under the frozen-gate procedure;
   deltas are recorded in an ADR follow-up entry.
5. The gate decision (stay frozen / unfreeze with scope) is re-recorded with
   the new evidence; silence is not an outcome.

## Acceptance Criteria

- [ ] >= 15 associative fixtures merged into the golden corpus, each with a
      documented hop path.
- [ ] Committed discriminative-power report: baseline per-channel and fused
      scores on the new slice, demonstrating headroom.
- [ ] Entity-BFS proxy arm and literal `graph_edges` traversal arm both
      evaluated on the slice; per-arm deltas recorded.
- [ ] ADR follow-up entry with the re-recorded gate decision, linked from
      #676.

## Edge Cases

- Fixtures that accidentally leak lexical overlap (a file path appearing in
  both query and target): the construction check measures token overlap
  between query and judged memory and rejects fixtures above a threshold.
- Entity extraction misses the linking entity: that is itself signal — the
  fixture stays, and the failure is attributed to the entity channel in the
  report rather than silently patched.
- The literal traversal arm requires populated `graph_edges` for the fixture
  corpus: fixture setup writes edges through the existing provenance-typed
  contract (`graph_contract.rs`), not ad-hoc inserts.

## Rollout Notes

Eval-only work inside the existing ADR contract; no production retrieval
change ships from this spec. The follow-up decision, if it unfreezes anything,
gets its own spec and flag.
