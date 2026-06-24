# Coding-Agent A/B Benchmark

Manual benchmark for issue #385. It compares the same coding tasks under:

- `no_memory`: no remem hooks or injected memory.
- `remem`: fixture evidence is saved into a temporary remem database, rendered through the SessionStart context path, and preloaded into `REMEM_CONTEXT.md`.
- `curated_file`: the same evidence is provided as a hand-curated `MEMORY.md`.

The first fixture is intentionally small and deterministic. It borrows the scoring shape of SWE-bench style patch tasks, but uses an inline repository so the harness can run from a clean checkout without Docker or external issue data. It should be expanded later with pinned real-repo tasks.

## Latest Baseline

Generated: 2026-06-25 01:44 CST

Runner: `codex-cli 0.142.0`, model `gpt-5.5`, `runs_per_condition=3`, 5 tasks, 45 total agent runs.

| Condition | Resolved | Resolution | Mean tokens | Mean wall time |
|---|---:|---:|---:|---:|
| `no_memory` | 3/15 | 20.0% | 390,003 | 133.6s |
| `remem` | 15/15 | 100.0% | 170,284 | 62.2s |
| `curated_file` | 15/15 | 100.0% | 146,840 | 60.5s |

Interpretation: remem now reaches the curated-file control's resolution rate on this fixture and strongly beats no-memory. It does not beat the curated file on token cost, so the stop-loss control remains active: the next benchmark slice should add broader real-repo tasks before making stronger product claims.

Reports:

- `eval/coding-bench/reports/baseline.json`
- `eval/coding-bench/reports/fix-remem-smoke.json`

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

Codex non-interactive MCP calls can be cancelled by the host. To keep the `remem` condition faithful but runnable, the harness still seeds a temporary remem database and uses the production SessionStart render path, then appends full seeded memory details to `REMEM_CONTEXT.md` as preloaded `get_observations` details. This avoids undercounting remem because of host MCP approval behavior rather than memory quality.

## Expansion Targets

Good next task sources:

- SWE-bench style real GitHub issue patch tasks, especially smaller or verified subsets: https://www.swebench.com/
- LiveCodeBench style fresh code-generation/self-repair tasks for contamination resistance: https://livecodebench.github.io/
- A remem-specific pinned real repo with hidden tests for memory-dependent architecture, policy, and regression constraints.
