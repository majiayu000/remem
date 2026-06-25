# Coding-Agent A/B Benchmark

Manual benchmark for issue #385. It compares the same coding tasks under:

- `no_memory`: no remem hooks or injected memory.
- `remem`: fixture evidence is saved into a temporary remem database, rendered
  through the SessionStart context path, and preloaded into `REMEM_CONTEXT.md`.
- `curated_file`: the same evidence is provided as a hand-curated `MEMORY.md`.

The v1 fixture is deterministic and public. It borrows the scoring shape of
SWE-bench style patch tasks, but uses an inline repository so the harness can
run from a clean checkout without Docker or external issue data. The pack has
16 memory-dependent tasks across eight categories, with a three-task smoke
subset for fast validation. Later versions should add pinned real-repo tasks.

## Isolated Baseline

Generated: 2026-06-25 19:16 CST

Runner: `codex-cli 0.142.1`, model `gpt-5.5`, `runs_per_condition=3`, 5 tasks,
45 total agent runs. This baseline predates the 16-task v1 fixture pack and
must be regenerated before publication.

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
| `remem` | 15/15 | 100.0% | 104,749 | 58.7s |
| `curated_file` | 15/15 | 100.0% | 94,017 | 62.8s |

Interpretation: remem matches curated-file resolution and strongly beats
no-memory on this first small memory-dependent fixture. Curated-file remains a
carefully maintained file baseline, so this does not prove remem beats a
manually curated `MEMORY.md`. The stop-loss control remains active: the next
benchmark slice should add broader real-repo tasks before making stronger
product claims.

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
hooks, user rules, or session log persistence. The runner also launches Codex
with a clean temporary HOME/CODEX_HOME and strips common host environment
variables. On macOS the harness wraps Codex in a host-read sandbox that denies
reads under the real HOME except the Codex install path plus temporary benchmark
run roots.

The `curated_file` condition intentionally includes a repo-local `MEMORY.md` in
each fixture checkout. Raw artifact scans may therefore contain `MEMORY.md`
references for that condition; host home, host `.codex`, auth files, virtualenvs,
and benchmark-private Codex homes must not appear.

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

Good next task sources:

- SWE-bench style real GitHub issue patch tasks, especially smaller or verified
  subsets: https://www.swebench.com/
- LiveCodeBench style fresh code-generation/self-repair tasks for contamination
  resistance: https://livecodebench.github.io/
- A remem-specific pinned real repo with hidden tests for memory-dependent
  architecture, policy, and regression constraints.
