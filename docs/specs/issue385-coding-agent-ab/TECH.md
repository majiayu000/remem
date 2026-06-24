# Coding-Agent A/B Benchmark Technical Spec

Status: Current contract
Issue: #385

## Current State

Current eval coverage is strong for internal memory behavior but does not run an
agent through real coding tasks:

- `remem eval` checks retrieval against `eval/golden.json`.
- `remem eval-extraction` checks transcript extraction fixtures.
- `remem eval-gates` combines deterministic retrieval, SessionStart injection,
  and extraction gates.
- `remem eval-e2e` exercises the local REST API against a temporary data
  directory.

The new benchmark must sit beside these checks. It measures end-to-end agent
outcomes, not only whether a memory result was retrieved.

## Proposed Layout

```text
src/eval/coding_bench.rs
src/eval/coding_bench/
  condition.rs
  fixture.rs
  runner.rs
  score.rs
  types.rs
eval/coding-bench/
  README.md
  fixtures/
    tasks.json
  reports/
    baseline.json
```

Implemented status: the native Rust runner and the first committed baseline are
present. The first fixture uses an inline Python repository for deterministic
clean-checkout runs; later versions should add pinned real-repo fixtures.

## CLI Contract

Add a command with an explicit manual-run posture:

```bash
remem eval-coding-bench \
  --fixture eval/coding-bench/fixtures/tasks.json \
  --runs-per-condition 3 \
  --json-out eval/coding-bench/reports/baseline.json
```

Required flags:

- `--fixture`: task-set definition.
- `--runs-per-condition`: minimum 3 for baseline publication.
- `--json-out`: report path.

Optional flags:

- `--condition no_memory|remem|curated_file`: run one condition for debugging.
- `--task <id>`: run one task for debugging.
- `--keep-workdirs`: preserve temporary workdirs after failure.
- `--model <name>` and `--provider <name>`: override runner defaults.
- `--dry-run`: validate fixtures and print the planned matrix without invoking
  an agent.

The command must refuse to run without an explicit `--json-out` path unless it
is `--dry-run`.

## Runner Architecture

Each run gets isolated state:

1. Create a temporary workdir from the pinned fixture repository.
2. Create a temporary `REMEM_DATA_DIR`.
3. Apply the selected memory condition.
4. Invoke the configured coding-agent runner with a bounded timeout.
5. Run the task scoring oracle.
6. Record artifacts and clean up unless `--keep-workdirs` is set.

The runner must pass command arguments as arrays, not shell-concatenated strings.
Any provider key must come from the environment or the provider's normal local
configuration. No secrets may be stored in fixtures or reports.

## Fixture Schema

`tasks.json` should contain:

```json
{
  "version": 1,
  "repo": {
    "kind": "git",
    "url": "https://example.com/org/repo.git",
    "rev": "fixed-sha"
  },
  "tasks": [
    {
      "id": "fix-parser-edge-case",
      "prompt": "Fix the parser edge case and add the focused regression test.",
      "timeout_ms": 180000,
      "allowed_paths": ["src/parser.rs", "tests/parser.rs"],
      "score": {
        "commands": [["cargo", "test", "--test", "parser"]]
      }
    }
  ]
}
```

The current fixture format supports objective scoring commands, hidden files
written after the agent run, and path constraints. Path constraints are not a
sandbox boundary; they are evaluated after the run to detect whether the agent
touched unauthorized files. Generated Python cache paths are ignored.

## Memory Seeding

The `remem` condition seeds a temporary database from committed fixture evidence,
not from private user memory. The implementation uses `save_memory` against a
temporary `REMEM_DATA_DIR`, then renders the production SessionStart context.
Because Codex non-interactive MCP detail calls can be cancelled by the host, the
benchmark appends full seeded memory details to `REMEM_CONTEXT.md` as preloaded
`get_observations` details.

The `curated_file` condition uses the fixture's `curated_context` text and writes
it to `MEMORY.md` in the temporary repository. It must be derived from the same
source evidence as the remem seed and reviewed as part of fixture changes.

The `no_memory` condition must disable remem hooks, MCP registration, and native
memory file injection for the temporary agent run.

## Report Schema

`baseline.json` must include:

```json
{
  "schema_version": 1,
  "generated_at_epoch": 0,
  "repo_rev": "fixed-sha",
  "remem_rev": "current-sha",
  "runner": {
    "provider": "codex-cli",
    "model": "example-model",
    "version": "runner-version"
  },
  "runs_per_condition": 3,
  "conditions": [
    {
      "name": "remem",
      "summary": {
        "resolution_rate": 0.0,
        "tokens_total_mean": 0.0,
        "tokens_total_stddev": 0.0,
        "turns_mean": 0.0,
        "wall_time_ms_mean": 0.0,
        "wall_time_ms_p95": 0.0
      },
      "runs": []
    }
  ]
}
```

Each run entry records:

- condition
- task id
- run index
- resolved boolean
- score command outputs or references
- token usage
- turn count
- wall time
- final head SHA or patch artifact
- unauthorized path changes
- failure reason

Reports must not include full prompts containing secrets, provider API keys, or
private user memory content.

## Verification

Initial implementation should use focused checks first:

```bash
remem eval-coding-bench --dry-run \
  --fixture eval/coding-bench/fixtures/tasks.json \
  --runs-per-condition 3 \
  --json-out /tmp/remem-coding-bench.json

remem eval-coding-bench \
  --fixture eval/coding-bench/fixtures/tasks.json \
  --runs-per-condition 3 \
  --runner codex \
  --model gpt-5.5 \
  --reasoning-effort medium \
  --ignore-budget \
  --json-out eval/coding-bench/reports/baseline.json
```

Before publishing or changing README claims, also run:

```bash
cargo test -q eval::golden --lib
cargo test -q eval::injection --lib
remem eval-gates --json-out /tmp/remem-eval-gates.json
```

The first benchmark runner may be manual-only because it can be slow and model
dependent. If it later enters CI, CI should validate fixture parsing and schema
stability by default, with full agent runs behind an explicit scheduled or
maintainer-triggered workflow.

## Latest Baseline

Generated on 2026-06-25 with `codex-cli 0.142.0`, `gpt-5.5`,
`runs_per_condition=3`, 5 tasks, and 45 total agent runs:

| Condition | Resolved | Resolution | Mean tokens | Mean wall time |
|---|---:|---:|---:|---:|
| `no_memory` | 3/15 | 20.0% | 390,003 | 133.6s |
| `remem` | 15/15 | 100.0% | 170,284 | 62.2s |
| `curated_file` | 15/15 | 100.0% | 146,840 | 60.5s |

Result: remem matches curated-file resolution and strongly beats no-memory on
this fixture. Curated file remains cheaper, so this is not evidence that remem
beats a carefully maintained `MEMORY.md`; it is evidence that remem's runtime
path can deliver the same task-resolution rate on this memory-dependent fixture.

## Failure Handling

Benchmark failures are data, not process failures, when the runner and scoring
oracle complete successfully. A lower remem score must be committed honestly in
the report and linked from the roadmap decision.

Process failures include:

- fixture cannot be checked out
- scoring command is invalid
- runner does not isolate `REMEM_DATA_DIR`
- report omits required metrics
- token accounting is missing without an explicit unsupported-provider note
- curated file contains task answers or information unavailable to remem

## Open Technical Decisions

- The exact agent runner abstraction and first supported provider.
- How to collect tokens reliably across Codex CLI, Claude Code, and future
  hosts.
- Whether the fixture repository should be generated in-repo or cloned from a
  pinned external repository.
- Whether `src/eval/coding_bench/` should own the runner or delegate to a
  standalone script under `eval/coding-bench/` for easier provider iteration.
