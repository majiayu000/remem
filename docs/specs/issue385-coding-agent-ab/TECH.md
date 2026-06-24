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
  artifact.rs
  condition.rs
  fixture.rs
  runner.rs
  score.rs
  types.rs
eval/coding-bench/
  README.md
  fixtures/
    tasks.json
    curated-context.md
    seed-events.json
  reports/
    baseline.json
```

If the first implementation needs a smaller slice, `eval/coding-bench/README.md`
and the JSON schema may land before the Rust runner, but the issue is not
complete until the command produces a baseline report from a clean checkout.

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
      "timeout_ms": 900000,
      "allowed_paths": ["src/parser.rs", "tests/parser.rs"],
      "score": {
        "commands": [["cargo", "test", "--test", "parser"]]
      }
    }
  ]
}
```

The fixture format must support objective scoring commands and path constraints.
Path constraints are not a sandbox boundary; they are evaluated after the run to
detect whether the agent touched unauthorized files.

## Memory Seeding

The `remem` condition seeds a temporary database from committed fixture evidence,
not from private user memory. The first implementation may seed memories through
public CLI or MCP surfaces; direct database writes are allowed only for fixture
setup code that already goes through migration-managed schemas and is documented
as test data loading.

The `curated_file` condition uses `fixtures/curated-context.md`. That file must
be derived from the same source evidence as the remem seed and reviewed as part
of fixture changes.

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
- `memory_contract_status`: `passed`, `failed`, or `not_applicable`
- `runtime_contract_failure` and `runtime_contract_failure_reason`
- score command outputs or references
- token usage, or `token_accounting_unsupported_reason` when the provider cannot
  expose token accounting
- turn count
- wall time
- final head SHA or patch artifact
- unauthorized path changes
- failure reason

For `remem` condition runs, the run entry must also include
`remem_contract_snapshot`. The snapshot is the benchmark handoff from
`docs/specs/current-memory-contracts/TECH.md` and must include:

- the full `current_memory_contracts` deterministic report used for the run;
- `contract_health`, including failing examples and contract warnings;
- citation precision and usage feedback coverage;
- injected, dropped, and abstained memory audit coverage;
- staleness/source-anchor handling;
- temporal fact eligibility checks.

`no_memory` and `curated_file` runs must set `memory_contract_status` to
`not_applicable` and must not carry a `remem_contract_snapshot`. This keeps the
three benchmark conditions comparable and prevents control runs from faking
remem runtime evidence.

Task failure and runtime contract failure are separate outcome dimensions. A
coding task can pass while the remem runtime contract fails; that run must set
`resolved=true` and `runtime_contract_failure=true` instead of hiding the
contract failure inside a generic task failure reason.

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
