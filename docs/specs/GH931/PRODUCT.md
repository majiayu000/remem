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
- Public wording is governed by a pre-registered claim registry with an
  automatic wording gate: `PASS` / `FAIL` / `INSUFFICIENT`, allowed and
  forbidden wording, and a supporting report hash. `INSUFFICIENT` wording must
  be explicitly directional. Thresholds lock before the first official run.

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
