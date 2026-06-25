# Coding-Agent A/B Benchmark

Manual benchmark for issue #385. It compares the same coding tasks under:

- `no_memory`: no remem hooks or injected memory.
- `remem`: fixture evidence is saved into a temporary remem database, rendered
  through the SessionStart context path, and preloaded into `REMEM_CONTEXT.md`.
- `curated_file`: the same evidence is provided as a hand-curated `MEMORY.md`.

The first fixture is intentionally small and deterministic. It borrows the
scoring shape of SWE-bench style patch tasks, but uses an inline repository so
the harness can run from a clean checkout without Docker or external issue data.
It should be expanded later with pinned real-repo tasks.

## Draft Baseline

Generated: 2026-06-25 01:44 CST

Runner: `codex-cli 0.142.0`, model `gpt-5.5`, `runs_per_condition=3`, 5 tasks,
45 total agent runs.

This run was generated before the runner started ignoring host Codex config,
rules, hooks, and session persistence. Treat it as report-shape evidence only
until it is regenerated with the isolated runner.

| Condition | Resolved | Resolution | Mean tokens | Mean wall time |
|---|---:|---:|---:|---:|
| `no_memory` | 3/15 | 20.0% | 390,003 | 133.6s |
| `remem` | 15/15 | 100.0% | 170,284 | 62.2s |
| `curated_file` | 15/15 | 100.0% | 146,840 | 60.5s |

Interpretation: do not use this draft run for public product claims until the
isolated-runner baseline is regenerated. The stop-loss control remains active:
the next benchmark slice should add broader real-repo tasks before making
stronger product claims.

Reports:

- `eval/coding-bench/reports/baseline.json`
- `eval/coding-bench/reports/fix-remem-smoke.json`

The JSON reports are the only benchmark outputs intended for source control.
Raw per-run `runner.stdout`, `runner.stderr`, score outputs, and `final.diff`
files are generated under `eval/coding-bench/reports/artifacts/` for local
audit and are intentionally ignored. Those files can include local runner paths
or host-specific tool output, so rerun the benchmark to regenerate them instead
of committing them.

## Artifact Contract

The current public benchmark contract requires every `remem` run artifact to
carry current-memory evidence. The contract helper in `src/eval/coding_bench`
defines the canonical fields:

- `remem_contract_snapshot`, built from the current-memory-contracts
  deterministic report;
- `memory_contract_status`: `passed`, `failed`, or `not_applicable`;
- `runtime_contract_failure` and `runtime_contract_failure_reason`;
- score command evidence, patch evidence, token metrics, turns, and wall time.

`no_memory` and `curated_file` runs must set `memory_contract_status` to
`not_applicable` and must not include remem contract evidence.

Runtime contract failure is separate from agent task failure. A run may solve
the coding task while still failing the remem runtime contract; reports must
preserve both facts instead of merging them into one failure reason.

## Commands

Dry run:

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

Focused smoke:

```bash
cargo run -- eval-coding-bench \
  --fixture eval/coding-bench/fixtures/tasks.json \
  --runs-per-condition 1 \
  --condition remem \
  --task slug-normalizer-contract \
  --runner codex \
  --model gpt-5.5 \
  --reasoning-effort medium \
  --ignore-budget \
  --keep-workdirs \
  --json-out /tmp/remem-coding-bench-smoke.json
```

## Current Caveat

Codex non-interactive MCP calls can be cancelled by the host. To keep the
`remem` condition faithful but runnable, the harness still seeds a temporary
remem database and uses the production SessionStart render path, then appends
full seeded memory details to `REMEM_CONTEXT.md` as preloaded `get_observations`
details. This avoids undercounting remem because of host MCP approval behavior
rather than memory quality.

The Codex runner uses `--ignore-user-config`, `--ignore-rules`, `--ephemeral`,
and `--disable hooks` so benchmark agents do not inherit the host's MCP servers,
hooks, user rules, or session log persistence.

## Expansion Targets

Good next task sources:

- SWE-bench style real GitHub issue patch tasks, especially smaller or verified
  subsets: https://www.swebench.com/
- LiveCodeBench style fresh code-generation/self-repair tasks for contamination
  resistance: https://livecodebench.github.io/
- A remem-specific pinned real repo with hidden tests for memory-dependent
  architecture, policy, and regression constraints.
