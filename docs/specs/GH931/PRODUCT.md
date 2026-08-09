# GH931 Flagship E2E Public Proof — Product Spec

Status: Current contract (harness scaffold landed; runner execution support and
official runs pending)
Issue: #931 (refs #384, #385, #849, #928)

## Problem

The current coding-bench public evidence has two limits: the `remem` condition
seeds fixture evidence directly and preloads full details into
`REMEM_CONTEXT.md` (not the real capture → extraction → promotion → retrieval
path), and `curated_file` is a near-oracle human upper bound built from gold
evidence. Neither can support the flagship product claim.

## Claim under test

In continuously changing real projects, remem reaches coding-task success
comparable to or better than a target-blind, time-budgeted human-maintained
`MEMORY.md`, at clearly lower human maintenance cost, without materially
increasing stale/irrelevant memory harm.

## Product decisions

- Primary conditions: `no_memory`, `curated_file_budgeted`, `remem_e2e`.
  All other conditions (including the renamed `remem_preloaded` and
  `curated_file_expert`) are diagnostic only and never claim-bearing.
- The only primary outcome is `resolved_rate`; maintenance cost, token/latency
  cost, memory harm, and citation quality are mandatory secondary reports.
- Every memory failure is attributed to exactly one of six stages (capture,
  extraction, consolidation, retrieval, context compilation, reader/use) via a
  fixed 12-enum taxonomy.
- Every remem-backed run binds its artifact to the exact persisted production
  SessionStart ContextAudit. Missing or hash/summary-invalid audit evidence is
  a runtime contract failure; non-remem controls mark the contract not
  applicable.
- Public wording is governed by a pre-registered claim registry with an
  automatic wording gate: `PASS` / `FAIL` / `INSUFFICIENT`, allowed and
  forbidden wording, and a supporting report hash. `INSUFFICIENT` wording must
  be explicitly directional. Thresholds lock before the first official run.

## Completion contract (implementation pending)

This current contract owns the completion requirements. The longer
`specs/GH931/` packet is supporting historical planning evidence, not a
workflow gate or authorization source.

- The claim-bearing matrix is exactly 144 keys in
  `issue385-v1/official-v1`: 16 tasks × 3 primary conditions × 3 runs.
  Smoke approvals must assign policy-derived, non-colliding
  `run_phase=smoke` namespaces; smoke artifacts can never become official
  evidence. Smoke and official work still share one cumulative-budget ledger.
- The registered outcome for one task/condition is the arithmetic mean of its
  exactly three pre-registered binary `resolved` values; majority-success and
  any-success aggregation are forbidden. A target-started timeout, crash, or
  scoring failure contributes `0`. A pre-target missing tuple or any
  integrity-invalid tuple makes the full matrix `INSUFFICIENT`; it is never
  imputed or removed from a denominator.
- Each primary comparison first subtracts the control three-run mean from the
  treatment three-run mean within each of the 16 paired tasks, then averages
  those 16 task differences and reports the absolute percentage-point effect.
  Every percentile-bootstrap replicate samples 16 task IDs with replacement
  and recomputes both three-run means and their paired difference for every
  sampled task; repetitions are never treated as independent clusters.
  Missing pair/hash evidence is `INSUFFICIENT`. The immutable registration
  locks the bootstrap algorithm/version, replicate count, seed, and 95%
  percentile-index rule before any official run.
- Every paired condition uses the same registered `evaluation_as_of`. Semantic
  capture/candidate/promotion timestamps, TTL/active-memory decisions, and all
  SessionStart, PromptSubmit, search/detail, temporal, staleness, usage, graph,
  rerank, and explain paths receive that clock. The benchmark MCP router
  exposes only `search` and `get_observations`; ambient Rust/SQLite time and
  every other MCP tool are rejected. Real security and duration clocks remain
  separate.
- `remem_e2e` uses the production capture → extraction → candidate/review →
  promotion → retrieval path. Any treatment-side human work is target-blind,
  frozen before target reveal, supervisor-timed, and included in maintenance
  cost. Its capture input is a canonical, hash-bound projection of allowlisted
  raw history events only. It flattens the registered `history_episodes` and
  their `raw_events` in literal nested-array order and derives one
  projection-wide `source_ordinal=0..N-1`; timestamps must be nondecreasing by
  ordinal, equal-second events are valid, and `event_id` is identity only,
  never an ordering key. It preserves every content/tool channel, with exactly
  one capture call and inserted row per raw event in strict ordinal order.
  Capture-visible project/session identity is opaque and task-neutral; summary,
  expected-memory, gold, target, and scorer fields cannot enter capture. Direct
  seed, preload, or target-aware repair is invalid.
- Target agents and tools receive no repository authority, benchmark/scorer
  files, or public network. Only the pinned Codex process may reach a private
  loopback provider adapter; it has no network and forwards bounded frames over
  supervisor-created pipes. Tool subprocesses cannot reach loopback or Unix
  sockets. Hidden scoring runs only after the target exits, under an independent
  scorer OS principal, process, and tree. The controller never imports or
  executes patched code; an untrusted code worker has no hidden mount and can
  exchange only bounded, schema-checked canonical JSON RPC. Stdout, exit zero,
  visible tests, or a worker-asserted result cannot produce PASS. Shared
  interpreter state, monkeypatch reachability, malformed RPC, or scorer failure
  fails closed.
- Live execution requires a default-branch, independently reviewed approval
  entry bound to an OS/security-owner-anchored host supervisor and exact
  binaries, profiles, fixtures, tuples,
  pricing, token/call/cost caps, ledger writer, two rulesets, and TUF/Rekor
  trust material.
  A validation-only mode must verify the real entry without starting an agent
  or provider call. A fixed root-owned supervisor obtains expected digests from
  authority, uses `openat(O_NOFOLLOW)` plus same-fd/same-handle execution, and
  signs an attestation with a caller-inaccessible key for every process.
- Ledger records are signed by a dedicated writer identity. One active GitHub
  ruleset restricts updates to that App; a second active ruleset has no bypass
  actors and blocks deletion/force-push while requiring signed commits. After
  every remote compare-and-swap, the ledger tip is externally anchored outside
  GitHub by an operator-signed Sigstore Rekor checkpoint discovered via
  pinned TUF trust metadata. Missing, inconsistent, rolled-back, forked-ledger,
  unsigned, wrong-writer, or ruleset/log-trust drift fails closed. Without a
  separately approved witness quorum, this does not claim detection of a
  malicious Rekor operator's self-consistent split view.
- Every flagship run is first frozen as a receipt-free immutable RFC 8785 JCS
  payload. It contains no terminal receipt, checkpoint, source-manifest/report
  hash, or other field derived from its own digest. The supervisor hashes and
  CAS-seals those bytes first; only then may the source manifest add a detached
  mapping from that payload digest to the terminal attestation and checkpoint
  receipt. Verification walks payload → ledger seal → checkpoint chain.
- Default report construction and verification are fully offline: they verify
  only the signed receipts and proofs bundled at execution time and make no
  claim about current authority freshness. A separate explicit network
  freshness invocation may emit a detached signed receipt binding the immutable
  report SHA-256, ledger tip, ruleset, TUF and Rekor digests, observation time,
  and expiry. Publication, issue closure, and release require an unexpired
  matching receipt; the freshness invocation never rewrites the report.
- `memory_hurt` and `stale_memory_followed` are computed only by a
  pre-registered, hashed, scorer-only closed causal classifier over sealed
  traces. Missing or ambiguous classification makes the claim
  `INSUFFICIENT`; it is never treated as no harm.
- Contract review, security review, live auth/cost approval, exact public
  wording, final review, merge, issue closure, and release remain separate
  human decisions. No scaffold or dry-run result authorizes a public outcome.

## Acceptance mapping (v1 scaffold)

| Issue acceptance item | Scaffold artifact |
|---|---|
| Rename `remem` → `remem_preloaded` | `eval/coding-bench/conditions.json`, README, spec supersession notes; src-side id rename is the tracked follow-up |
| Real `remem_e2e` condition | Condition definition with forbidden shortcuts and isolation rules; runner execution support is the tracked src follow-up |
| `curated_file_budgeted` protocol | `eval/coding-bench/curated-file-budgeted-protocol.md` + curator log schema |
| Claim registry + wording gate | `eval/claims/registry.json`, `eval/claims/claim_gate.py` |
| Dry-run and schema validation | `python3 eval/coding-bench/validate_schemas.py`, `claim_gate.py --self-test`, `cargo run -- bench coding --suite issue385-v1 --dry-run` |
| Directional vs publishable distinction | `Directional evidence:` prefix rule enforced by the gate |

Paired statistical reports, maintenance-cost reporting from real curator logs,
and stop-loss measurement require executed runs and are out of scope for the
scaffold.

## Non-goals

Unchanged from the issue: no new retrieval channel, no LLM judge as primary
outcome, no immediate 96-task real-repo dataset, no success-definition changes
to pass the benchmark.
