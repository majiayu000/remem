# Cross-Host Continuity Benchmark (cross-host-v1)

Infrastructure for issue #935: can history produced in one host (Claude Code
or Codex) be reliably used by the other host on a new continuation task, and
does remem beat target-host native memory or an exported handoff file?

Status: `infrastructure_only_no_runs`. This directory contains the versioned
charter, schemas, task skeletons, leak scanner, and an offline dry-run runner.
No benchmark run has been executed and no result may be cited from here.

## Layout

- `benchmark-charter.json`: versioned charter — directions, conditions,
  handoff protocol, metrics, stop-loss thresholds, and the public claim gate.
- `schemas/cross-host-task.schema.json`: task definition contract.
- `schemas/cross-host-run.schema.json`: per-run artifact contract, including
  the attribution block (source/target host, session/event/memory/context
  refs, per-ref origin classification, scope and validity at target time).
- `tasks/claude-to-codex/`, `tasks/codex-to-claude/`: 12 task skeletons per
  direction, one per required category. All are `status: "skeleton_todo"`
  with explicit `todo` lists; text fields describe design intent, not
  fixture data. Hidden tests and deterministic fixtures are follow-up work.
- `examples/`: one valid and one invalid run artifact used to exercise the
  run schema. Schema examples only, not benchmark results.
- `scripts/schema_validate.py`: stdlib-only validator for the JSON Schema
  subset these schemas use.
- `scripts/scan_artifacts.py`: leak scanner. Fails artifacts that reference
  real host HOME paths, host session stores (`~/.claude/projects`,
  `~/.codex/sessions`, ...), auth/credential material, or declared
  benchmark-private roots. Isolated HOMEs under temp roots are allowed.
- `scripts/run_dry.py`: offline dry-run. Validates charter, tasks, coverage,
  and optional artifacts; prints the planned run matrix. Never launches a
  host agent.

## Directions and conditions

Two directions, reported separately (merged-average-only reporting is
forbidden by the claim gate):

```text
Claude Code -> remem -> Codex
Codex -> remem -> Claude Code
```

Primary conditions: `no_memory`, `target_host_native`, `exported_file`,
`remem_shared`. Diagnostic conditions: `source_host_native_imported`,
`remem_preloaded`, `oracle_handoff`, `full_transcript`,
`remem_without_host_native_import`, `remem_with_host_native_import`.

Each direction needs at least 12 tasks covering the 12 required categories,
with at least 3 runs per task/condition.

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
python3 eval/cross-host/scripts/run_dry.py
python3 eval/cross-host/scripts/schema_validate.py \
  eval/cross-host/examples/run-artifact-valid.json \
  eval/cross-host/schemas/cross-host-run.schema.json
python3 eval/cross-host/scripts/scan_artifacts.py <artifact-dir> \
  --private-root <benchmark-private-home>
```

## Follow-up (not in this directory yet)

- Deterministic fixture repositories and hidden tests per task.
- Real host execution harness reusing the `eval/coding-bench` isolation
  policy (temporary HOME, stripped env, host-read sandbox).
- Paired bootstrap analysis and claim-gate wiring into `bench report`.
