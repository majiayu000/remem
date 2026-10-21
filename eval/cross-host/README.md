# Cross-Host Continuity Benchmark (cross-host-v1, v2 compatibility dry-run)

Infrastructure for issue #935: can history produced in one host (Claude Code
or Codex) be reliably used by the other host on a new continuation task, and
does remem beat target-host native memory or an exported handoff file?

Status: `infrastructure_only_no_runs`. This directory contains the versioned
charter, schemas, task skeletons, leak scanner, an offline dry-run runner, and
a `cross-host-v2` compatibility plan mode that maps the legacy
`remem_shared` label to `remem_shared_startup`. No benchmark run has been
executed and no result may be cited from here.

## Layout

- `benchmark-charter.json`: versioned charter — directions, conditions,
  handoff protocol, metrics, stop-loss thresholds, and the public claim gate.
- `schemas/cross-host-task.schema.json`: task definition contract.
- `schemas/cross-host-run.schema.json`: strict `cross-host-v1` run artifact
  shape. `schemas/cross-host-run-v2.schema.json` is the separately versioned
  `cross-host-v2` compatibility shape. Neither schema is an executable runner.
  Both include the attribution block (source/target host,
  session/event/memory/context refs, per-ref origin classification, scope and
  validity at target time), but their suite/version/condition labels do not
  cross-validate.
- `tasks/claude-to-codex/`, `tasks/codex-to-claude/`: 12 logical scenarios
  repeated in both directions, covering all required categories. Each of the
  24 task definitions has at least two concrete chronological source episodes,
  a continuation request, hidden test files and score commands. Paired
  directions are not 24 independent statistical units. All tasks remain
  `status: "skeleton_todo"` with TODOs for the remaining v2 task/scorer and
  production execution boundaries.
- `fixtures/smoke.json`: the existing shared inline-Python repository recipe,
  scorer-private reference repairs and two known-wrong repairs per scenario.
  The name is retained; it now contains all 12 scenarios. The four registered
  architecture-decision/failed-attempt tasks remain the first planned live
  smoke milestone. Source episodes and fixtures are deterministic test data,
  not claims that either host has executed them.
- `scripts/test_smoke_fixtures.py`: the existing offline check now runs all
  24 tasks. It requires 24 base failures, 24 reference successes and 48
  wrong-repair failures through behavioral assertions, never import/runtime
  errors masquerading as a failed outcome. It prepares actual temporary Git
  histories with fixed author/time and distinct stable/next branches, plus
  two distinct tenant directories both named atlas. Temporary Git commands
  disable user hooks/signing and ignore inherited Git configuration without
  changing global settings. The fixed preparation is part of the offline
  fixture recipe; a future host runner must reproduce it before target work.
  Hidden tests, gold facts and repair recipes are scorer-private, never
  target-visible repository files. This check launches no host/provider and
  establishes no cross-host outcome, production filtering or scope isolation.
- `examples/`: v1 and v2 compatibility schema fixtures, including invalid
  inputs used to exercise diagnostics. Fixtures only, not benchmark results.
- `scripts/schema_validate.py`: stdlib-only validator for the JSON Schema
  subset these schemas use.
- `scripts/scan_artifacts.py`: leak scanner. Fails artifacts that reference
  real host HOME paths, host session stores (`~/.claude/projects`,
  `~/.codex/sessions`, ...), auth/credential material, or declared
  benchmark-private roots. Isolated HOMEs under temp roots are allowed.
- `scripts/run_dry.py`: offline dry-run. Validates charter, tasks, coverage,
  and optional artifacts; prints the planned run matrix. `--suite-version
  cross-host-v2` emits the v2 compatibility plan. `executable_ready` is always
  `false`, including if every task is manually marked ready, because the full
  v2 runtime contract and the shared executable runner from #931 are not
  implemented here. Never launches a host agent.

## Directions and conditions

Two directions, reported separately (merged-average-only reporting is
forbidden by the claim gate):

```text
Claude Code -> remem -> Codex
Codex -> remem -> Claude Code
```

Primary v1 conditions: `no_memory`, `target_host_native`, `exported_file`,
`remem_shared`. The v2 compatibility plan renames the remem arm to
`remem_shared_startup` to make clear that it measures only startup selection,
not the complete SessionStart-plus-interactive-MCP product path. Diagnostic
conditions: `source_host_native_imported`, `remem_preloaded`,
`oracle_handoff`, `full_transcript`, `remem_without_host_native_import`,
`remem_with_host_native_import`.

V1 requires at least 12 tasks per direction. The v2 compatibility plan
requires exactly 12 unique task IDs per direction, covering the 12 required
categories, with 3 planned runs per task/condition.

## Handoff isolation

Source and target hosts use independent HOME/config/session stores. The
target phase must not read the source host session store, and every counted
run artifact must pass the leak scanner (`leak_scan_passed: true`). Stop-loss
is absolute: `wrong_project_injection = 0`, `wrong_user_injection = 0`,
`source_private_session_leak = 0`, `stale_memory_followed <= 1%`,
`memory_hurt <= 2%`. Any boundary leak fails the release claim regardless of
resolved-rate gains.

## Commands

```bash
python3 eval/cross-host/scripts/schema_validate.py --self-test
python3 eval/cross-host/scripts/scan_artifacts.py --self-test
python3 eval/cross-host/scripts/test_run_dry.py
python3 eval/cross-host/scripts/test_smoke_fixtures.py
python3 eval/cross-host/scripts/run_dry.py
python3 eval/cross-host/scripts/run_dry.py \
  --suite-version cross-host-v2 \
  --json-out /tmp/cross-host-v2-dry-run.json
python3 eval/cross-host/scripts/schema_validate.py \
  eval/cross-host/examples/run-artifact-valid.json \
  eval/cross-host/schemas/cross-host-run.schema.json
python3 eval/cross-host/scripts/schema_validate.py \
  eval/cross-host/examples/run-artifact-v2-plan-valid.json \
  eval/cross-host/schemas/cross-host-run-v2.schema.json
python3 eval/cross-host/scripts/scan_artifacts.py <artifact-dir> \
  --private-root <benchmark-private-home>
```

## Follow-up (not in this directory yet)

- Complete v2 causal oracles, pre-run scorer commitments, foreign-project
  canaries and source seals for all 24 task definitions. Bind stale challenges
  to production state and per-run pre-filter matches; the offline stale and
  scope output checks do not demonstrate production filtering or isolation.
  These repository-owned capabilities remain unimplemented independently of
  the model/environment/budget needed for authorized live runs.
- Shared executable runner from #931, followed by the GH935-specific real-host
  harness using the `eval/coding-bench` isolation policy (temporary HOME,
  stripped env, host-read sandbox).
- Paired bootstrap analysis and claim-gate wiring into `bench report`.
