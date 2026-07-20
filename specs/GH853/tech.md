# Tech Spec

## Linked Issue

GH-853

## Product Spec

[`product.md`](product.md)

<!-- specrail-planned-changes
{"version":1,"issue":853,"complete":true,"paths":["CHANGELOG.md","Cargo.lock","Cargo.toml","README.md","docs/ARCHITECTURE.md","docs/adr/2026-07-09-graph-gate-associative-followup.md","docs/graph-contract.md","docs/specs/README.md","eval/graph-decision/report.json","npm/remem/package.json","plugins/remem/.codex-plugin/plugin.json","plugins/remem/runtimes/remem-releases.json","server.json","specs/GH853/product.md","specs/GH853/tasks.md","specs/GH853/tech.md","src/eval/graph_decision.rs","src/eval/golden/run.rs","src/eval/weight_grid.rs","src/retrieval.rs","src/retrieval/graph.rs","src/retrieval/graph/query.rs","src/retrieval/graph/tests.rs","src/retrieval/graph/traverse.rs","src/retrieval/graph/types.rs","src/retrieval/search/memory/tests.rs","src/retrieval/search/memory/text.rs","src/retrieval/search/memory/text/graph.rs","src/retrieval/search/memory/weights.rs"],"spec_refs":["specs/GH853/product.md","specs/GH853/tech.md","specs/GH853/tasks.md"]}
-->

## Codebase Context

| Area | Files | Current behavior | Why relevant |
| --- | --- | --- | --- |
| Standard RRF search | `src/retrieval/search/memory/text.rs`, `weights.rs` | Timed FTS/entity/fact/temporal/vector/fallback channels; no graph reader | Supplies eligible seeds, fusion, explain timing, and disabled reasons |
| Typed graph storage | `src/memory/graph_contract.rs`, `docs/graph-contract.md` | Validated trusted/diagnostic edges with typed endpoints and validity intervals | Traversal must preserve trust and endpoint semantics |
| Existing entity expansion | `src/retrieval/entity/graph/` | Two-hop proxy over `memory_entities`; not a `graph_edges` reader | Useful bounded-query pattern but not the required source |
| Graph decision gate | `src/eval/graph_decision.rs` | Compares standard vs entity proxy and hardcodes `graph_edges_evaluated=false` | Must gain a literal arm and associative primary gate |
| Golden corpus | `src/eval/golden/run.rs`, `eval/golden.json` | Contains 15 pre-registered associative `hop_path` fixtures, but seeds no trusted graph edges | Must seed evidence without deriving edges from answers at query time |

## Design

### Bounded traversal core

Add `src/retrieval/graph/` with closed request/outcome/path types, fixed
parameterized SQL, and deterministic traversal. The request contains seed IDs,
search filters, a reference time, and positive caps. The core:

1. validates caps and returns `no_seed` before SQL for an empty seed set;
2. checks graph table availability/row count and distinguishes `empty_graph`;
3. reads trusted, currently valid edges adjacent to memory seeds;
4. emits eligible memory targets for direct `supersedes` and expands
   `mentions`/`touches_file` through one shared entity/file node;
5. records `extracted_from` and diagnostic hints only in aggregate diagnostics;
6. filters target memories in SQL using the same project/type/branch/current
   predicates as standard search, then lets the existing suppression filter
   enforce user-visible suppression policy;
7. deduplicates seed/cycle/diamond targets by best path and sorts by hop count,
   path priority, descending minimum edge confidence, seed rank, then memory ID.

No new public `Any`-typed API or stringly edge contract is introduced.

### Search channel

After FTS and vector channels have run, collect only their filtered hit IDs in
rank order and call the traversal core. Convert ranked graph targets into a
`graph_traversal` `NamedChannel`. Empty outcomes become stable
`disabled_reason` values; execution errors propagate. `time_result` records a
`graph_traversal` phase. `SearchWeights.graph` controls the RRF contribution.

The default weight is non-zero only after the literal gate generated from the
same code head passes. Weight-grid distance/candidate generation includes the
new dimension so evaluation does not silently ignore it.

### Eval fixture and gate

`seed_fixture_corpus` keeps ordinary fixture insertion unchanged. A graph-only
seeding helper used by the literal arm resolves pre-registered `hop_path`
source/target topic keys, creates the entity/file node and minimum valid
captured-event/candidate/operation provenance, then inserts both trusted edges
through `insert_graph_edge`. It never reads expected evidence while executing a
query and never mutates production databases.

Extend `GraphDecisionMode` with `LiteralGraph`. The standard arm uses graph
weight zero; the literal arm uses the candidate production weight. Reports add
literal metrics, associative primary deltas, real two-edge counts, empty/error
status, non-associative regression checks, p95 latency, and
`graph_edges_evaluated=true`. `ensure_graph_decision_gate` fails unless every
required gate passes. The committed report and ADR are updated only from a
fresh run on the implementation head.

## Product-to-Test Mapping

| Product invariant | Implementation area | Verification |
| --- | --- | --- |
| `B-001`, `B-003` | search seed selection + traversal | FTS/vector-only seed and one/two-edge path tests |
| `B-002`, `B-012` | graph query/path classifier | trusted/diagnostic/validity/extracted-from tests |
| `B-004` | target SQL + suppression filter | project/branch/type/stale/suppressed matrix |
| `B-005` | traversal caps/dedupe/order | cap, cycle, diamond, reverse insertion tests |
| `B-006`, `B-007` | outcome + `NamedChannel` explain | empty/no-seed/no-expansion/error/timing tests |
| `B-008`-`B-010` | graph decision evaluator/report | literal associative gain and non-regression gate tests |
| `B-011` | fixed SQL/read-only core | injection sentinel and DB-change-count tests |

## Data Flow

```text
query
  -> eligible suppression-filtered FTS/vector seed IDs
  -> bounded trusted graph_edges traversal (same SQLite connection)
  -> eligible deterministic graph memory IDs
  -> graph_traversal RRF channel + timing/explain
  -> existing fusion, confidence gate, load, pagination
```

The eval literal arm seeds only its in-memory fixture database through the real
typed/provenance graph writer, then invokes the same traversal core.

## Alternatives

- Reuse `memory_entities`: rejected because it does not test or consume the
  persisted typed graph contract.
- Generic recursive CTE or unbounded BFS: rejected because fan-out and path
  semantics become difficult to audit.
- PPR in this PR: rejected as optional phase-two work that needs its own
  quality/latency comparison after bounded traversal is established.
- Context wiring: rejected because SessionStart has separate fusion and is not
  in GH-853 acceptance.

## Risks

- Security: shared entity/file nodes can leak scope; target eligibility is
  applied before IDs leave the traversal core, and suppression is re-applied.
- Compatibility: adding a `SearchWeights` field changes serialized weight
  objects; serde default preserves older inputs.
- Performance: graph fan-out is capped and indexed; cap violations return an
  error rather than partial results.
- Evaluation integrity: edges come from immutable `hop_path` fixture metadata,
  not dynamic expected-answer lookup.
- Maintenance: graph writer coverage may be sparse, so safe empty reasons are
  part of the contract.

## Test Plan

- [ ] `cargo test -q retrieval::graph --lib`
- [ ] `cargo test -q retrieval::search::memory::tests --lib`
- [ ] `cargo test -q eval::graph_decision --lib`
- [ ] `cargo test -q eval::golden --lib`
- [ ] `cargo run -- eval-graph-decision --json-out eval/graph-decision/report.json --json`
- [ ] `cargo fmt --check`; `cargo check`; `cargo clippy --all-targets -- -D warnings`; `cargo test`
- [ ] `cargo run -- eval-extraction --json --check-baseline`
- [ ] `cargo run -- eval-gates --json-out /tmp/remem-eval-gates.json`
- [ ] version sync, SpecRail workflow/spec checks, and full PR preflight

## GH900 Follow-up: Report Evidence Fingerprint

PR #899 wired the production graph channel but committed
`eval/graph-decision/report.json` with metrics and a date string only — no
content fingerprint of `eval/golden.json` or the evaluator/retrieval sources, so
a stale report could survive source/dataset changes silently (GH-900).

### Mechanism

`src/eval/graph_decision/evidence_fingerprint.rs` computes a deterministic,
length-prefixed SHA-256 fingerprint (`len(path) || path || len(content) ||
content`, folded over a stable sorted file list) over:

- the dataset: `eval/golden.json`;
- the evaluator/retrieval sources that determine the result:
  `src/eval/golden.rs`, `src/eval/golden/run.rs`, `src/eval/golden/types.rs`,
  `src/eval/graph_decision.rs`, `src/retrieval/graph.rs`,
  `src/retrieval/graph/query.rs`, `src/retrieval/graph/traverse.rs`,
  `src/retrieval/graph/types.rs`, `src/retrieval/search/memory/text/graph.rs`.

The report gains an `evidence_fingerprint` block (`algorithm`,
`dataset_sha256`, `implementation_sha256`, `combined_sha256`, and the explicit
per-input `inputs` list with `path`/`role`/`byte_len`/`sha256`). The fingerprint
module is intentionally not hashed: it does not affect the graph-decision result,
and hashing itself would be circular (any edit, even test-only, would force a
regeneration). Fingerprint-logic drift is still caught because a changed
`compute` yields different digests than the committed report.

### Guard

`checked_in_graph_decision_report_matches_generated_fingerprint` regenerates the
report from the live tree and asserts the committed `evidence_fingerprint`
matches, plus the deterministic decision/check/metric-delta fields. Latency
fields (`deltas.p95_latency_ms`, per-query timings) are excluded because they
vary per run. A stale report fails loudly with "fingerprint is stale; regenerate
eval/graph-decision/report.json" (no warn-and-pass). Regenerate with
`cargo run -- eval-graph-decision --json-out eval/graph-decision/report.json`.

### Completed regression slices (GH-900 test matrix)

`src/retrieval/graph/tests.rs`: `touches_file` two-hop, parameterized
memory_type/branch/status eligibility, seed/candidate/edge-scan cap fail-closed
(joining the existing degree-cap test), stable cycle/diamond ordering, read-only
row invariance plus ignored-edge decode, non-positive-limit validation error, and
stable per-status `disabled_reason`. The traversal's "invalid bridge kind" and
"unknown trust" branches are defense-in-depth: the v034 migration enforces edge
structure (CHECK + `edge_trust IN ('trusted','diagnostic_hint')` + node-existence
triggers), so those states are unreachable through the graph contract; the
reachable error behavior is the fail-closed limit/cap validation.

`src/retrieval/search/memory/text/graph.rs`: FTS/vector-only seed selection
(dedupe + cap), post-suppression channel construction, missing-table channel
`disabled_reason` plus timing, and real two-hop graph hits with a suppression row.

## Rollback

Set the graph weight to zero or revert the search-channel wiring and report/ADR
update. The traversal core and existing graph rows are read-only/additive, so no
database rollback is needed. The GH900 fingerprint is additive; rolling it back
removes the `evidence_fingerprint` report field and its guard test.
