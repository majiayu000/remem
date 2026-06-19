# Coding-Agent A/B Benchmark Product Spec

Status: Current contract
Issue: #385

## Problem

remem has deterministic retrieval, injection, extraction, and local API evals,
but those gates do not prove the product claim users care about: whether a
coding agent completes real engineering tasks better when remem is available.

The README currently avoids claiming that remem beats a carefully maintained
`MEMORY.md` on coding tasks. This benchmark defines the evidence required
before that claim can be made.

## Goal

Produce a reproducible end-to-end benchmark that compares one fixed coding-task
set under three memory conditions:

1. `no_memory`: the agent receives only the task prompt and repository files.
2. `remem`: the agent receives context through remem's normal install/runtime
   path.
3. `curated_file`: the agent receives a hand-curated context file generated from
   the same source material that remem is allowed to use.

The benchmark reports task-resolution rate, token usage, turn count, wall time,
and variance across at least three runs per condition.

## Non-Goals

- Do not replace the existing golden retrieval, injection, extraction, or
  eval-gates checks.
- Do not claim statistical significance from the first small sample.
- Do not use the user's real `~/.remem` database or private memory as benchmark
  input.
- Do not tune prompts, fixtures, or curated files after seeing per-condition
  results without recording a new benchmark version.
- Do not let the benchmark auto-merge code changes, push branches, or write
  GitHub comments.

## Product Contract

The first public report is directional evidence only. It may say that one
condition performed better in the committed fixture set, but it must not claim
general superiority unless a later benchmark version adds enough task diversity
and repeated runs to justify that stronger statement.

The curated-file condition is a standing falsification control. If a small
hand-maintained file ties or beats remem on task-resolution rate with lower cost
and no material usability downside, the M6 roadmap must record that result as a
stop-loss signal and pivot the next roadmap slice toward ergonomics instead of
more retrieval machinery.

## Benchmark Subject

The fixture should use a small, deterministic software repository or pinned
worktree that can be checked out from a clean environment. Tasks should look
like real coding-agent work:

- fix a failing test with a clear root cause
- implement a small feature that depends on prior design context
- preserve an existing architectural constraint
- avoid a known regression or forbidden pattern
- update a focused test or documentation artifact when required

Each task must have an objective scoring oracle, such as command success,
expected file diff constraints, or a machine-readable evaluator. Human review
can be an additional annotation, but it cannot be the only pass/fail gate.

## Memory Conditions

### No Memory

The agent runs without remem hooks, remem MCP, `MEMORY.md`, or curated context.
The prompt may include only the task, repo path, and standard agent operating
instructions.

### remem

The agent runs with remem enabled through the normal supported integration path.
The benchmark must seed a temporary remem database from fixture evidence, then
inject or retrieve memory through the same public surfaces that users run:

- hooks/context path for coding-agent session startup when practical
- MCP or CLI retrieval only when that mirrors supported user behavior
- a temporary `REMEM_DATA_DIR`, never the user's real data directory

### Curated File

The agent receives one committed context file produced from the same fixture
evidence available to remem. The file must be human-readable and small enough
to be a realistic manually maintained project memory file.

The curated file is not allowed to contain task answers, exact target diffs, or
information that remem was not also permitted to ingest.

## Metrics

Every run records:

- `resolved`: whether the task passed its objective oracle
- `tokens_input`, `tokens_output`, and `tokens_total`
- `turns`
- `wall_time_ms`
- `commands_run`
- `final_head_sha` or patch artifact id
- `failure_reason` when unresolved

The report aggregates by condition:

- resolution rate
- mean and standard deviation for token usage
- mean and standard deviation for turns
- mean and p95 wall time
- per-task outcome table

## Acceptance Criteria

- A baseline report is committed with three conditions and at least three runs
  per condition.
- The benchmark runs from a clean checkout with one documented command.
- The benchmark uses a temporary data directory and does not read or write the
  user's real remem database.
- The curated-file control and stop-loss rule are documented in the harness
  README and baseline report.
- Each task has an objective scoring oracle checked by the runner.
- Result artifacts are committed as JSON and include enough environment metadata
  to reproduce the run.

## Open Product Decisions

- Which model/provider is the first supported runner allowed to use.
- Whether the first fixture repo should be this repo, a small synthetic repo, or
  a pinned external public repo.
- Whether token accounting comes from provider usage APIs, model CLI logs, or a
  wrapper that records both prompt and completion usage.
- Whether the first public report should be manually run only or included as a
  non-blocking CI artifact.
