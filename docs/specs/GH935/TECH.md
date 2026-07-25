# Cross-Host Continuity Benchmark Tech Spec

Status: Current contract (v1 infrastructure)
Issue: #935

## Layout

Everything lives under `eval/cross-host/`; v1 touches no `src/` code.

| Path | Contract |
|---|---|
| `benchmark-charter.json` | Versioned charter (`charter_version: cross-host-v1`, `status: infrastructure_only_no_runs`): directions, conditions, handoff protocol, metrics, stop-loss, claim gate. |
| `schemas/cross-host-task.schema.json` | Task definition: direction/category enums, source-host episodes with expected memory facts, target task, score block (hidden tests), gold memory, `status` + `todo` lifecycle. |
| `schemas/cross-host-run.schema.json` | Per-run artifact: condition enum (4 primary + 6 diagnostic), metrics, `handoff_isolation` (const-false shared session store, `leak_scan_passed`), and the required `attribution` block with per-ref `origin` in {`remem_canonical_capture`, `host_native_import`, `generated_projection`, `manual_save`} plus `scope_at_target` / `validity_at_target`. |
| `tasks/claude-to-codex/`, `tasks/codex-to-claude/` | 12 skeleton tasks per direction, one per required category, ids `cc2cx-*` / `cx2cc-*`. |
| `scripts/schema_validate.py` | Stdlib JSON Schema subset validator (`--self-test`); unsupported keywords raise so schemas cannot drift ahead of it. |
| `scripts/scan_artifacts.py` | Leak scanner (`--self-test`): host HOME paths, `~/.claude` / `~/.codex` session stores, auth/credential material, `--private-root` markers; isolated HOMEs under temp roots are allowed. |
| `scripts/run_dry.py` | Offline dry-run: validates charter/tasks/artifacts, enforces >=12 tasks and full category coverage per direction, direction/host consistency, `skeleton_todo` vs `ready` invariants, condition-specific attribution rules, and prints the planned run matrix. |
| `examples/` | One valid and one invalid run artifact exercising the run schema; explicitly not results. |

## Task lifecycle

Tasks start as `status: "skeleton_todo"` with a non-empty `todo` list; text
fields are design intent, never fabricated fixture data. Promotion to
`ready` requires real deterministic fixtures, hidden test files, non-empty
`score.commands`, and an empty `todo` list; `run_dry.py` enforces both sides.

## Verification

```bash
python3 eval/cross-host/scripts/schema_validate.py --self-test
python3 eval/cross-host/scripts/scan_artifacts.py --self-test
python3 eval/cross-host/scripts/run_dry.py
```

## Follow-up implementation work

- Deterministic fixture repos + hidden tests per task; flip tasks to `ready`.
- Host execution harness reusing `eval/coding-bench` isolation (temp
  HOME/CODEX_HOME, stripped env, macOS host-read sandbox) for both hosts.
- Paired bootstrap + claim-gate wiring into `bench report` / `bench verify`.
- exported_file generation/maintenance cost measurement protocol.
