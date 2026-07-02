# Associative Multi-Hop Fixtures Technical Spec

Status: Current contract
Date: 2026-07-02

Tracking:
- Spec/tracking issue: #676

## Existing Implementation Facts

- Golden corpus and harness: `eval/golden.json` (10 multi_hop fixtures,
  saturated per the ADR), `src/eval/golden/`.
- ADR: `docs/adr/2026-06-12-graph-decision-gate.md` — decision
  `keep_graph_edges_frozen_pending_literal_eval`; entity-BFS proxy evaluated
  `safe_to_wire=false`; follow-ups explicitly list fixture expansion and a
  literal traversal eval arm.
- Graph surfaces: `graph_edges` / `graph_file_nodes` / `graph_candidates`
  tables (v031/v033/v034) with the typed provenance contract in
  `src/memory/graph_contract.rs`; entity-BFS proxy under
  `src/retrieval/entity/graph/`; explicit multi-hop search under
  `src/retrieval/search_multihop/`.
- Entity extraction feeds the entity channel
  (`src/retrieval/entity/`), weight 1.25 in fusion
  (`src/retrieval/search/memory/weights.rs`).

## Design Rules

- Fixtures are constructed adversarially against lexical/semantic channels
  and validated mechanically, not by eyeball.
- Evidence before wiring: no production retrieval change in this work.
- Both arms run the identical slice and procedure; deltas are comparable.
- Fixture graph edges go through `graph_contract.rs` types so the literal arm
  tests the real contract.

## Proposed Design

### Fixture construction

Extend `eval/golden.json` with a `query_type: "associative"` slice:

1. Each fixture is a triple: query text, intermediate memory (shares the
   linking entity with the target), target memory (judged relevant), plus
   `hop_path` documentation (`query -> <entity> -> target`).
2. Linking entity classes: file path, crate name, error signature, issue
   number — at least 3 fixtures per class, >= 15 total.
3. Mechanical overlap check in the golden loader tests: token overlap
   (after the existing query tokenization in `src/retrieval/query_expand/`)
   between query and target content must be below a documented threshold;
   embedding-cosine between query and target (current provider) is recorded
   in the report for transparency.
4. Fixture setup writes `graph_edges` rows for the hop links via
   `graph_contract.rs` constructors with fixture provenance ids.

### Discriminative-power baseline

New report step in the golden harness: run the default channel set on the
associative slice only, emit per-channel and fused R@5 / nDCG. Acceptance
requires recorded headroom (fused R@5 comfortably below saturation; exact
ceiling recorded in the report rather than hardcoded here).

### Eval arms

Reusing the frozen-gate procedure from the ADR:

- Arm 1 — entity-BFS proxy: existing `src/retrieval/entity/graph/` path via
  `--multi-hop`.
- Arm 2 — literal traversal: a new eval-only retrieval arm that seeds from
  the query's entities, traverses `graph_edges` (1-2 hops, typed edges,
  provenance intact), and scores reached memories into the fusion set. Lives
  under `src/eval/` or a feature-gated module so it cannot ship into
  production retrieval accidentally.
- Both arms report delta vs baseline on the associative slice and on the full
  golden set (to catch regressions elsewhere), same metrics as the ADR run.

### Decision recording

Append an ADR follow-up entry (same file, dated section) with: slice
composition, baseline scores, per-arm deltas, and the re-recorded decision.
Cross-link from #676.

## Product-to-Test Mapping

| Product invariant | Implementation area | Verification |
| --- | --- | --- |
| P1 associative construction | golden.json + loader | loader test enforces triple + hop_path presence |
| P1/edge overlap rejection | overlap check | test: crafted leaky fixture rejected |
| P2 verified headroom | baseline report | committed report artifact; harness test asserts report generation |
| P3 documented hop path | fixture schema | loader test requires hop_path |
| P4 both arms, same slice | eval arms | harness test runs both arms on the slice in test profile |
| P5 decision recorded | ADR follow-up | PR review checklist item; link on #676 |

## Data Flow

Fixture triples -> golden DB build (memories + typed graph_edges) -> baseline
run (default channels) -> arm runs (entity-BFS, literal traversal) -> metrics
JSON + committed report -> ADR follow-up decision.

## Alternatives Considered

- Synthesizing associative queries from real project DBs: rejected for the
  gate (non-reproducible); possible later as a companion study.
- Testing only the entity-BFS proxy again: rejected; the ADR's open question
  is specifically whether literal typed-edge traversal outperforms the proxy.
- Building the literal arm as a production channel behind a zero weight:
  rejected; eval-only isolation is safer and the ADR requires evidence before
  any wiring.

## Risks

- Security: none; eval-only.
- Compatibility: golden.json schema gains optional fields; loader stays
  backward-compatible with existing fixtures.
- Performance: 1-2 hop traversal over a fixture-sized edge set is trivial;
  no production path affected.
- Maintenance: fixture quality is the whole point — the mechanical overlap
  check and per-fixture hop docs keep them honest as the corpus evolves.

## Test Plan

- [ ] Unit tests: loader validation (triple, hop_path, overlap threshold),
      literal-arm traversal on a toy edge set.
- [ ] Integration test: full associative slice through baseline + both arms
      in the test profile, JSON report shape asserted.
- [ ] Manual verification: run the full procedure once, read the per-fixture
      failures, and write the ADR follow-up with the recorded deltas.

## Rollback Plan

The slice and arms are additive eval surfaces; removing the slice from
golden.json and the eval-only arm module reverts everything. The ADR decision
history is append-only and stays.
