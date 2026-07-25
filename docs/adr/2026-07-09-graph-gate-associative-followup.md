# ADR Follow-up: Graph Gate Stays Frozen After Running the Associative Eval Arms

Date: 2026-07-09

## Status

Accepted. Follow-up to
[`docs/adr/2026-06-12-graph-decision-gate.md`](2026-06-12-graph-decision-gate.md)
(“Keep `graph_edges` Retrieval Frozen Pending a Literal Graph Eval”).

The freeze recorded below was superseded on 2026-07-19 by the GH-853
same-head literal decision appended at the end of this ADR.

## Context

The 2026-06-12 gate deferred a wire-or-freeze decision for `graph_edges`
retrieval until two follow-up eval arms ran against a harder fixture slice:

- an **entity-BFS proxy** arm (the existing `memory_entities` + FTS mention
  expansion path), and
- a **literal `graph_edges` traversal** arm.

PR #695 landed the associative multi-hop slice (15 fixtures across `file_path`,
`crate`, `error_signature`, and `issue_number` entities, zero query-target
token overlap) and its committed baseline report at
`eval/associative-multihop/baseline.json`, but explicitly deferred the eval
arms and this ADR re-record (`omitted_followups`:
`entity_bfs_proxy_delta`, `literal_graph_edges_traversal`,
`adr_decision_followup`). This follow-up records the outcome of running those
two arms on that slice. No fixtures, schema, migrations, or production
retrieval wiring were changed.

## Facts (measured this session)

### Committed associative baseline (single-channel / standard fused retrieval)

`[source: remem eval-associative-baseline, exit 0]`

```
remem eval-associative-baseline - slice=associative k=5
fixtures: 15 max_query_target_shared_tokens=0
baseline fused: hit@5=0.000 recall@5=0.000 nDCG@10=0.000 evidence@5=0.000
headroom:      hit@5=1.000 recall@5=1.000 nDCG@10=1.000 evidence@5=1.000
```

This reproduces the committed `eval/associative-multihop/baseline.json`
(`baseline_fused` all `0.0`, `headroom` all `1.0`). By construction the
associative slice has zero query-target lexical overlap, so single-channel
retrieval scores `0.0` and the whole `1.0` is headroom.

### Entity-BFS proxy arm on the associative slice

`[source: remem eval-graph-decision --json-out <report> --json]`, per-query
summaries filtered to `slice == "associative"`:

| Arm | associative queries | evidence_recall (matched/expected) | two-hop expansions | entities discovered | result_count |
| --- | --- | --- | --- | --- | --- |
| standard | 15 | 0 / 15 = **0.0** | n/a | n/a | 0 for all |
| entity-BFS proxy | 15 | 0 / 15 = **0.0** | 0 (all `hops = 1`) | 0 | 0 for all |

**Entity-BFS proxy delta on the associative slice = 0.0** (0.0 − 0.0), against a
benefit threshold of `0.05`
`[source: report.benefit_threshold = 0.05]`. The proxy discovered no bridge
entities and performed no second hop on any associative query, so it retrieves
exactly what standard retrieval does: nothing.

The same command's whole-corpus gate output (unchanged from 2026-06-12):
`decision = keep_graph_edges_frozen_pending_literal_eval`,
`graph_edges_evaluated = false`, `benefit_threshold_met = false`,
`safe_to_wire_entity_bfs = false`, `all_checks_passed = false`. The command
exits non-zero **by design** because the gate refuses to pass while the
entity-BFS proxy regresses non-multi-hop precision; the JSON report is written
before the gate bails.

### Literal `graph_edges` traversal arm

Outcome: **not runnable at eval seed** — recorded as the arm result, not forced.
Two concrete, independent reasons:

1. **No production `graph_edges` retrieval/traversal function exists.** Every
   production read of the table is an integrity check by id, not a
   query-driven traversal:
   `SELECT COUNT(*) FROM graph_edges` and
   `SELECT edge_trust FROM graph_edges WHERE id = ?1`
   `[source: src/memory/graph_contract.rs:591, 611, 777, 788]`. Every
   `FROM graph_edges` that resembles a BFS/expansion lives in tests only
   `[source: rg "FROM graph_edges" over src/, non-test hits are only the
   integrity reads above]`.
2. **`graph_edges` is empty at eval seed.** `seed_fixture_corpus` inserts
   memories via `insert_memory_full`, which only calls
   `refresh_memory_entities` (populating `entities`/`memory_entities`) and
   never inserts an edge
   `[source: src/eval/golden/run.rs:392-445; src/memory/store/write.rs:146,186]`.
   The only production writer, `insert_graph_edge`, is reached solely through
   candidate promotion (`src/graph_candidate/mod.rs:389`), which requires a
   `candidates` row plus an `operation_log` id built from a `captured_events`
   provenance chain `[source: src/graph_candidate/mod.rs:355-392]`. None of
   `captured_events`, `candidates`, or `operation_log` are populated by the
   fixture seed, so no trusted edge can exist to traverse.

Issue #676 forbids unfreezing/wiring graph traversal in this issue (“evidence
first”), so building a production traversal function to force this arm to run
is out of scope. Recording the non-runnable outcome with its cause is the
intended result.

## Inferences

- [based on: entity-BFS delta 0.0 + no discovered entities on any associative
  query; confidence: high] The entity-BFS proxy provides **no** retrieval
  signal on associative bridges as they are currently seeded, because the
  associative fixtures encode source→entity→target hops through topic keys and
  synthetic content that share no lexical tokens with the query, and the proxy
  still bottoms out on FTS/entity-mention matching. The `1.0` headroom is real
  headroom, but the entity-BFS proxy does not close any of it.
- [based on: absent production traversal fn + empty seed table; confidence:
  high] A trustworthy literal `graph_edges` traversal arm cannot exist until
  (a) a production traversal function is written and (b) the eval seed builds
  trusted edges through the real provenance chain. Both are explicitly deferred
  by #676.

## Decision (re-recorded)

**`graph_edges` retrieval stays FROZEN.** The gate’s benefit threshold
(`0.05` multi-hop evidence-recall gain) is not cleared: the measured
entity-BFS proxy delta on the associative slice is `0.0`, and the literal
traversal arm produced no evidence because it is not runnable at seed. The
2026-06-12 decision (`remain_frozen_pending_literal_eval`) is unchanged.

## Consequences

- `graph_edges` remains a typed graph/provenance contract, not a retrieval
  channel. #332, #334, #335 stay frozen.
- The associative slice now has a recorded, reproducible eval-arm result, not
  just a baseline. Any future attempt to unfreeze graph retrieval must first
  land a production `graph_edges` traversal function and a seed path that
  builds trusted edges through the provenance chain, then show a
  pre-registered gain above the `0.05` threshold on this slice.

## Suggestions

- [assumption: a future graph channel is still wanted; alternative: drop the
  graph-retrieval roadmap entirely] Before re-running these arms, extend the
  fixture seed to build trusted `graph_edges` through the real
  `captured_events → candidates → operation_log → insert_graph_edge` chain, so
  the literal arm has data to traverse. Risk/cost: this couples eval seeding to
  the capture pipeline and must preserve project/branch/owner/type/stale
  filters; it should land behind explicit configuration first (per the
  2026-06-12 follow-ups).

## Verification (this session)

- `cargo fmt --check` — exit 0.
- `cargo check --message-format=short` — `Finished dev` (exit 0).
- `remem eval-associative-baseline` — exit 0, output quoted above.
- `remem eval-graph-decision --json-out <report> --json` — writes the report,
  then exits non-zero by design (gate freeze); associative per-query summaries
  extracted from that report are quoted above.

## 2026-07-19 GH-853 Literal Graph Decision

Status: Accepted. Supersedes the earlier freeze for the bounded production
channel described in `specs/GH853/`; it does not authorize PPR or context
injection wiring.

GH-853 added a shared SQLite-only traversal core, seeded the existing 15
pre-registered associative fixtures through the trusted provenance contract,
and ran standard (`graph=0`) and literal graph arms from the same code and
dataset. The committed `eval/graph-decision/report.json` records:

- associative evidence recall@5: `0.0 -> 1.0` (`delta=1.0`, threshold `0.05`)
- associative real two-edge queries: `15/15`
- non-associative recall/evidence/nDCG deltas: `0.0 / 0.0 / 0.0`
- scope leaks: `0`
- literal overall p95 latency below the `1000ms` budget
- `checks.safe_to_wire_literal_graph=true` and `checks.all_checks_passed=true`

Decision: `wire_literal_graph_traversal`. Standard memory search may use the
bounded `graph_traversal` RRF channel with eligible FTS/vector seeds. Only
trusted, currently valid `supersedes`, `mentions`, and `touches_file` paths are
rankable; diagnostic hints and `extracted_from` do not produce memory
candidates. Empty/no-seed/no-expansion states remain explicit disabled reasons,
and execution errors fail closed.

PPR, graph databases, schema changes, and direct SessionStart/context traversal
remain outside this decision and require separate evidence before adoption.
