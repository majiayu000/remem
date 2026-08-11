# Coding-Agent A/B Benchmark

Manual benchmark for issues #385 and #931. Issue #931 promotes this harness to
the flagship public proof: end-to-end remem versus a budgeted human baseline
versus no memory.

## Conditions

Machine-readable registry: `eval/coding-bench/conditions.json`, validated by
`eval/coding-bench/validate_schemas.py` against
`eval/coding-bench/schemas/conditions.schema.json`.

### Primary conditions (claim-bearing)

- `no_memory`: no remem hooks, MCP, SessionStart injection, `MEMORY.md`, or
  host-native memory. Only current code and the target task.
- `curated_file_budgeted`: target-blind, time-budgeted human-curated
  `MEMORY.md`. Protocol: `eval/coding-bench/curated-file-budgeted-protocol.md`.
  Every run attaches a curator log matching
  `eval/coding-bench/schemas/curator-log.schema.json`.
- `remem_e2e`: the real product path — raw session/tool evidence →
  captured_events → extraction_tasks → observations/candidates →
  review/promotion policy → memories/projections → SessionStart/MCP retrieval
  → coding agent. Direct gold-memory seeding and full-evidence
  `REMEM_CONTEXT.md` preloading are forbidden. Full execution requires a
  configured remem LLM provider key; dry-run and schema validation must pass
  offline.

### Diagnostic conditions (localization only, never claim-bearing)

- `remem_seeded_sessionstart`: fixture evidence saved into a temporary remem
  database, selected and rendered
  through the production SessionStart path, and the exact audited output written
  unchanged to `REMEM_CONTEXT.md`. It is retrieval-dependent and is not
  comparable with the historical `remem_preloaded` baseline.
- `curated_file_expert`: the former `curated_file` condition — unbudgeted,
  gold-evidence-derived `MEMORY.md`; near-oracle human upper bound.
- `oracle_evidence`, `remem_oracle_retrieval`, `full_history`,
  `remem_no_enrichment`, `remem_fts_only`: see `conditions.json`.

Runner status: the Rust runner (`src/eval/coding_bench`) implements `no_memory`
and `remem_seeded_sessionstart` under their stable ids, plus
`curated_file_expert` under the legacy CLI id `curated_file`. The remaining
curated id rename and `remem_e2e` / `curated_file_budgeted` execution support
are tracked as src-side follow-ups of #931; `conditions.json` records
per-condition `runner_status` so drift is visible.

## Isolated Baseline (predates #931 renames)

Generated: 2026-06-25 19:16 CST

Runner: `codex-cli 0.142.1`, model `gpt-5.5`, `runs_per_condition=3`, 5 tasks,
45 total agent runs. This baseline predates the 16-task v1 fixture pack and the
#931 condition renames (its reports record the legacy `remem` and
`curated_file` ids) and must be regenerated before publication.

This run was generated from clean source at remem revision
`c6a46aec3fe44c8a256138d839ebeea396b6cdb7` with `source_dirty=false`. The
Codex runner used an isolated temporary HOME/CODEX_HOME, ignored host Codex
config/rules/hooks/session persistence, stripped host virtualenv/env leakage,
and on macOS denied reads under the real host home except the Codex install path
and temporary benchmark run roots. Runs are marked failed if runner output
shows host home or benchmark-private Codex home access.

| Condition | Resolved | Resolution | Mean tokens | Mean wall time |
|---|---:|---:|---:|---:|
| `no_memory` | 2/15 | 13.3% | 115,373 | 75.7s |
| `remem_preloaded` (as `remem`) | 15/15 | 100.0% | 104,749 | 58.7s |
| `curated_file_expert` (as `curated_file`) | 15/15 | 100.0% | 94,017 | 62.8s |

Interpretation: preloaded remem matches expert curated-file resolution and
strongly beats no-memory on this first small memory-dependent fixture. Both
non-control conditions are upper bounds: neither exercises the real capture →
extraction → promotion → retrieval path, and the curated file was neither
target-blind nor budgeted. The #931 primary matrix exists to close exactly
this gap before making stronger product claims.

Reports:

- `eval/coding-bench/reports/baseline.json`
- `eval/coding-bench/reports/fix-remem-smoke.json`

The JSON reports are the only benchmark outputs intended for source control.
Raw per-run `runner.stdout`, `runner.stderr`, score outputs, and `final.diff`
files are generated under `eval/coding-bench/reports/artifacts/` for local
audit and are intentionally ignored. Those files can include local runner paths
or host-specific tool output, so rerun the benchmark to regenerate them instead
of committing them.

## Primary Metrics

The only primary outcome is `resolved_rate`. Also reported:

- FAIL_TO_PASS / PASS_TO_PASS, compile success, wrong file modified, timeout;
- tokens per resolved task, wall time per resolved task;
- human maintenance minutes / 100 sessions (from curator logs);
- memory_helped / memory_hurt, stale_memory_followed,
  irrelevant_memory_distracted, missing_relevant_memory;
- citation precision / recall, time_to_first_relevant_file,
  repeated_failed_action_count.

## Memory Failure Decomposition

Every memory failure must be attributed to exactly one stage. The
stage-to-enum mapping is machine-readable in `conditions.json`
(`failure_stages`) and validated by `validate_schemas.py`:

| Stage | Failure enums |
|---|---|
| Capture | `evidence_not_captured` |
| Extraction | `durable_fact_missed`, `unsupported_claim_saved`, `wrong_scope` |
| Consolidation | `update_not_applied`, `conflict_not_detected`, `stale_memory_not_invalidated` |
| Retrieval | `relevant_memory_missing`, `irrelevant_memory_selected` |
| Context compilation | `context_budget_dropped` |
| Reader/use | `retrieved_but_ignored`, `memory_misapplied` |

## Claim Gate

Public wording is governed by `eval/claims/registry.json` and enforced by
`python3 eval/claims/claim_gate.py check`. Pre-registered v1 thresholds:

```text
remem_e2e vs no_memory:
  resolved_rate improvement >= 10pp, 95% CI lower bound > 0

remem_e2e vs curated_file_budgeted:
  non-inferiority margin <= 3pp
  human maintenance time reduction >= 70%

stop-loss:
  memory_hurt <= 2%
  stale_memory_followed <= 1%
```

Every claim is `PASS`, `FAIL`, or `INSUFFICIENT` with allowed/forbidden
wording and a supporting report hash. `INSUFFICIENT` wording must be prefixed
`Directional evidence:` — that is the mechanical line between directional and
publishable evidence. Thresholds may be adjusted before the first official run
and never retroactively.

## Artifact Contract

The current public benchmark contract requires every remem-backed run artifact
to carry current-memory evidence. The contract helper in
`src/eval/coding_bench` defines the canonical fields:

- `remem_contract_snapshot`, built from the current-memory-contracts
  deterministic report and the exact persisted SessionStart ContextAudit;
- `context_audit_status`: `verified`, `contract_failure`, or
  `not_applicable`, plus a diagnostic failure reason when applicable;
- the payload-free ContextAudit snapshot: injection run id, bundle/plan and
  policy versions, plan/audit hashes, an injection-run binding hash, degraded
  mode, candidate/selection/drop counts, token budget/estimate, truncation
  reason, and canonical audit JSON;
- `memory_contract_status`: `passed`, `failed`, or `not_applicable`;
- `runtime_contract_failure` and `runtime_contract_failure_reason`;
- `memory_contract`, with injected memory ids, cited/used memory ids, citation
  precision/recall, stale used count, irrelevant injection count, missing
  relevant memory count, `memory_helped`, and `memory_hurt`;
- score command evidence, patch evidence, token metrics, turns, and wall time.

`no_memory` and curated-file runs must set both `memory_contract_status` and
`context_audit_status` to `not_applicable` and must not include remem contract
evidence or memory attribution. `curated_file_budgeted` runs must additionally
attach the curator log artifact and a `MEMORY.md` hash matching
`final_file_sha256`.

The verifier canonicalizes the embedded ContextAudit JSON, dispatches on the
artifact's supported plan and ContextAudit bundle schema versions, recomputes
its SHA-256 and the domain-separated injection binding, rejects unknown v1
audit fields, checks every summary field, and derives candidate/selected/drop
counts from the audit entries. Condition setup also compares the snapshot with
the persisted `injection_run_id`. A missing or invalid audit is a runtime
contract failure even when the coding task resolves.

Runtime contract failure is separate from agent task failure. A run may solve
the coding task while still failing the remem runtime contract; reports must
preserve both facts instead of merging them into one failure reason.

Task `failure_reason` is a fixed enum, not free text:

- `test_failure`
- `timeout`
- `compile_failure`
- `wrong_file_modified`
- `ignored_memory`
- `missing_memory`
- `stale_memory_followed`
- `irrelevant_memory_distracted`
- `over_context_budget`
- `agent_hallucinated_memory`
- `oracle_inconclusive`

Reports aggregate `memory_failure_counts` separately from the full
`failure_counts` map so memory-specific failures are not mixed into ordinary
coding failures.

## Commands

Offline harness validation (no LLM key, no network):

```bash
python3 eval/coding-bench/validate_schemas.py
python3 eval/claims/claim_gate.py check
python3 eval/claims/claim_gate.py --self-test
```

Full v1 dry run:

```bash
cargo run -- bench coding \
  --suite issue385-v1 \
  --dry-run \
  --json-out /tmp/remem-issue385-v1-dry-run.json
```

Smoke subset dry run:

```bash
cargo run -- bench coding \
  --suite issue385-v1 \
  --task-set smoke \
  --dry-run \
  --json-out /tmp/remem-issue385-v1-smoke-dry-run.json
```

Legacy direct dry run:

```bash
cargo run -- eval-coding-bench \
  --fixture eval/coding-bench/fixtures/tasks.json \
  --runs-per-condition 3 \
  --json-out /tmp/remem-coding-bench.json \
  --dry-run
```

Full baseline:

```bash
cargo run -- eval-coding-bench \
  --fixture eval/coding-bench/fixtures/tasks.json \
  --runs-per-condition 3 \
  --runner codex \
  --model gpt-5.5 \
  --reasoning-effort medium \
  --ignore-budget \
  --json-out eval/coding-bench/reports/baseline.json
```

Focused smoke (legacy runner id until the src-side rename lands):

```bash
cargo run -- eval-coding-bench \
  --fixture eval/coding-bench/fixtures/tasks.json \
  --runs-per-condition 1 \
  --condition remem_seeded_sessionstart \
  --task slug-normalizer-contract \
  --runner codex \
  --model gpt-5.5 \
  --reasoning-effort medium \
  --ignore-budget \
  --keep-workdirs \
  --json-out /tmp/remem-coding-bench-smoke.json
```

## Current Caveat

Codex non-interactive MCP calls can be cancelled by the host. The
`remem_seeded_sessionstart` diagnostic condition therefore seeds a temporary
remem database and writes the production SessionStart output to
`REMEM_CONTEXT.md`. The runner does not append seeded memory bodies: the file
remains byte-for-byte aligned with the persisted ContextAudit. This condition
is still diagnostic-only under #931 because it directly seeds gold-derived
memories instead of exercising capture, extraction, and promotion. It is not
comparable with the historical `remem_preloaded` 15/15 result. MCP availability
issues in `remem_e2e` count as real failures with a stage attribution.

The Codex runner uses `--ignore-user-config`, `--ignore-rules`, `--ephemeral`,
and `--disable hooks` so benchmark agents do not inherit the host's MCP servers,
hooks, user rules, or session log persistence. The runner also launches Codex
with a clean temporary HOME/CODEX_HOME and strips common host environment
variables. On macOS the harness wraps Codex in a host-read sandbox that denies
reads under the real HOME except the Codex install path plus temporary benchmark
run roots.

The curated-file conditions intentionally include a repo-local `MEMORY.md` in
each fixture checkout. Raw artifact scans may therefore contain `MEMORY.md`
references for those conditions; host home, host `.codex`, auth files,
virtualenvs, and benchmark-private Codex homes must not appear.

## Fixture Pack

`eval/coding-bench/fixtures/tasks.json` is the public v1 task pack. Each task
records:

- category and smoke/full membership;
- history episodes with expected memory facts;
- target prompt, allowed paths, and forbidden paths;
- deterministic oracle commands and hidden test files;
- required and forbidden patch patterns checked on added diff lines;
- gold required/forbidden memory facts plus supporting event ids.

The required category coverage is two tasks each for prior decisions, prior bug
root causes, stale-memory avoidance, negative constraints, workstream
continuity, multi-hop project context, user-context relevance, and
conflict/ambiguity handling.

## Expansion Targets

Issue #931 phase two: at least 12 pinned real repositories, at least 96 target
tasks across Rust / Python / TypeScript / Go, 10-30 history episodes per
target, hidden tests or deterministic oracles as the outcome judge. Good task
sources:

- SWE-bench style real GitHub issue patch tasks, especially smaller or verified
  subsets: https://www.swebench.com/
- LiveCodeBench style fresh code-generation/self-repair tasks for contamination
  resistance: https://livecodebench.github.io/
- A remem-specific pinned real repo with hidden tests for memory-dependent
  architecture, policy, and regression constraints.
