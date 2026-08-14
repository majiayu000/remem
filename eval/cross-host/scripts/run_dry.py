#!/usr/bin/env python3
"""Offline dry-run for the cross-host continuity benchmark (issue #935).

Validates the charter, every task definition, and any run artifacts against the
suite schemas, checks direction/category coverage, and prints the planned run
matrix. It never launches a host agent.

Usage:
  run_dry.py                         # validate charter + tasks, print plan
  run_dry.py --suite-version cross-host-v2
  run_dry.py --artifacts DIR         # additionally validate run artifacts
  run_dry.py --json-out FILE         # write the dry-run report as JSON
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from schema_validate import load_json, validate  # noqa: E402

SUITE_ROOT = Path(__file__).resolve().parent.parent
TASK_SCHEMA = SUITE_ROOT / "schemas" / "cross-host-task.schema.json"
CHARTER = SUITE_ROOT / "benchmark-charter.json"
SUITE_V1 = "cross-host-v1"
SUITE_V2 = "cross-host-v2"
SUITE_CHOICES = (SUITE_V1, SUITE_V2)
RUN_SCHEMAS = {
    SUITE_V1: SUITE_ROOT / "schemas" / "cross-host-run.schema.json",
    SUITE_V2: SUITE_ROOT / "schemas" / "cross-host-run-v2.schema.json",
}
V2_TASKS_PER_DIRECTION = 12
V2_CONDITION_ALIASES = {
    "remem_shared": "remem_shared_startup",
}
V2_REQUIRED_SOURCE_NATIVE_IMPORT_CONDITIONS = [
    "remem_without_host_native_import",
    "remem_with_host_native_import",
]
TASK_DIRS = {
    "claude_to_codex": SUITE_ROOT / "tasks" / "claude-to-codex",
    "codex_to_claude": SUITE_ROOT / "tasks" / "codex-to-claude",
}
HOST_BY_DIRECTION = {
    "claude_to_codex": ("claude_code", "codex"),
    "codex_to_claude": ("codex", "claude_code"),
}


def normalize_condition(condition: object, suite_version: str) -> object:
    if suite_version == SUITE_V2 and isinstance(condition, str):
        return V2_CONDITION_ALIASES.get(condition, condition)
    return condition


def primary_conditions(charter: dict, suite_version: str) -> list[str]:
    conditions = [
        normalize_condition(condition, suite_version)
        for condition in charter["primary_conditions"]
    ]
    return [condition for condition in conditions if isinstance(condition, str)]


def diagnostic_conditions(charter: dict, suite_version: str) -> list[str]:
    normalized = [
        normalize_condition(condition, suite_version)
        for condition in charter.get("diagnostic_conditions", [])
    ]
    conditions = [condition for condition in normalized if isinstance(condition, str)]
    if suite_version == SUITE_V2:
        for condition in V2_REQUIRED_SOURCE_NATIVE_IMPORT_CONDITIONS:
            if condition not in conditions:
                conditions.append(condition)
    return conditions


def validate_tasks(
    charter: dict, errors: list[str], suite_version: str
) -> dict[str, list[dict]]:
    task_schema = load_json(TASK_SCHEMA)
    required_categories = set(charter["task_requirements"]["required_categories"])
    min_tasks = charter["task_requirements"]["min_tasks_per_direction"]
    tasks_by_direction: dict[str, list[dict]] = {}
    seen_ids: set[str] = set()

    for direction, task_dir in TASK_DIRS.items():
        tasks: list[dict] = []
        for path in sorted(task_dir.glob("*.json")):
            task = load_json(path)
            rel = path.relative_to(SUITE_ROOT)
            for err in validate(task, task_schema):
                errors.append(f"{rel}: {err}")
            task_id = task.get("id")
            if isinstance(task_id, str):
                if task_id in seen_ids:
                    errors.append(f"{rel}: duplicate task id {task_id!r}")
                seen_ids.add(task_id)
            if task.get("direction") != direction:
                errors.append(f"{rel}: direction {task.get('direction')!r} does not match directory")
            expected_hosts = HOST_BY_DIRECTION[direction]
            if (task.get("source_host"), task.get("target_host")) != expected_hosts:
                errors.append(f"{rel}: source/target hosts do not match direction {direction}")
            if task.get("status") == "skeleton_todo" and not task.get("todo"):
                errors.append(f"{rel}: skeleton_todo task must list todo items")
            if task.get("status") == "ready":
                if task.get("todo"):
                    errors.append(f"{rel}: ready task must have an empty todo list")
                if not task.get("score", {}).get("commands"):
                    errors.append(f"{rel}: ready task must define score commands")
                if not task.get("score", {}).get("hidden_files"):
                    errors.append(f"{rel}: ready task must define hidden test files")
            tasks.append(task)
        tasks_by_direction[direction] = tasks

        if suite_version == SUITE_V2:
            unique_ids = {
                task["id"]
                for task in tasks
                if isinstance(task.get("id"), str) and task.get("id")
            }
            if len(tasks) != V2_TASKS_PER_DIRECTION or len(unique_ids) != V2_TASKS_PER_DIRECTION:
                errors.append(
                    f"{direction}: cross-host-v2 requires exactly "
                    f"{V2_TASKS_PER_DIRECTION} tasks with "
                    f"{V2_TASKS_PER_DIRECTION} unique ids; found {len(tasks)} tasks "
                    f"and {len(unique_ids)} unique ids"
                )
        elif len(tasks) < min_tasks:
            errors.append(f"{direction}: {len(tasks)} tasks, charter requires {min_tasks}")
        covered = {t.get("category") for t in tasks}
        for missing in sorted(required_categories - covered):
            errors.append(f"{direction}: missing required category {missing!r}")
    return tasks_by_direction


def validate_artifacts(
    artifact_dir: Path,
    errors: list[str],
    suite_version: str,
    tasks_by_direction: dict[str, list[dict]],
    runs_per_task_condition: int,
) -> int:
    run_schema = load_json(RUN_SCHEMAS[suite_version])
    tasks_by_id = {
        task["id"]: task
        for tasks in tasks_by_direction.values()
        for task in tasks
        if isinstance(task.get("id"), str) and task.get("id")
    }
    seen_v2_tuples: dict[tuple[object, object, object, object], Path] = {}
    count = 0
    for path in sorted(artifact_dir.rglob("*.json")):
        count += 1
        try:
            artifact = load_json(path)
        except ValueError as exc:
            errors.append(str(exc))
            continue
        for err in validate(artifact, run_schema):
            errors.append(f"{path}: {err}")
        if not isinstance(artifact, dict):
            continue
        artifact_suite = artifact.get("suite")
        if artifact_suite != suite_version:
            errors.append(
                f"{path}: artifact suite {artifact_suite!r} does not match "
                f"requested suite {suite_version!r}"
            )
        if suite_version == SUITE_V2:
            direction = artifact.get("direction")
            task_id = artifact.get("task_id")
            condition = artifact.get("condition")
            run_index = artifact.get("run_index")
            if (
                isinstance(direction, str)
                and isinstance(task_id, str)
                and isinstance(condition, str)
                and isinstance(run_index, int)
                and not isinstance(run_index, bool)
            ):
                identity = (direction, task_id, condition, run_index)
                previous_path = seen_v2_tuples.get(identity)
                if previous_path is not None:
                    errors.append(
                        f"{path}: duplicate artifact tuple {identity!r}; "
                        f"first seen in {previous_path}"
                    )
                else:
                    seen_v2_tuples[identity] = path
            task_id = artifact.get("task_id")
            task = tasks_by_id.get(task_id) if isinstance(task_id, str) else None
            if task is None:
                errors.append(f"{path}: task_id {task_id!r} is not registered in the task matrix")
            else:
                for field in ("direction", "source_host", "target_host"):
                    if artifact.get(field) != task.get(field):
                        errors.append(
                            f"{path}: artifact {field} {artifact.get(field)!r} does not match "
                            f"task {task_id!r} value {task.get(field)!r}"
                        )
            run_index = artifact.get("run_index")
            if (
                isinstance(run_index, int)
                and not isinstance(run_index, bool)
                and not 0 <= run_index < runs_per_task_condition
            ):
                errors.append(
                    f"{path}: run_index {run_index} is outside the registered range "
                    f"0..{runs_per_task_condition - 1}"
                )
        condition = artifact.get("condition")
        attribution_value = artifact.get("attribution", {})
        attribution = attribution_value if isinstance(attribution_value, dict) else {}
        if condition == "no_memory":
            for key in ("promoted_memory_refs", "selected_context_item_refs", "used_refs"):
                if attribution.get(key):
                    errors.append(f"{path}: no_memory run must have empty attribution.{key}")
        attribution_condition = {
            SUITE_V1: "remem_shared",
            SUITE_V2: "remem_shared_startup",
        }[suite_version]
        if condition == attribution_condition:
            for key in ("source_session_refs", "captured_event_refs", "promoted_memory_refs"):
                if not attribution.get(key):
                    errors.append(
                        f"{path}: {attribution_condition} run must record attribution.{key}"
                    )
        isolation_value = artifact.get("handoff_isolation", {})
        isolation = isolation_value if isinstance(isolation_value, dict) else {}
        if not isolation.get("leak_scan_passed", False):
            errors.append(f"{path}: leak_scan_passed must be true for a counted run")
    return count


def task_diagnostic_id(task: dict, index: int) -> str:
    task_id = task.get("id")
    if isinstance(task_id, str) and task_id:
        return task_id
    return f"<missing-id:{index}>"


def build_plan(charter: dict, tasks_by_direction: dict[str, list[dict]], suite_version: str) -> dict:
    runs = charter["task_requirements"]["runs_per_task_condition"]
    conditions = primary_conditions(charter, suite_version)
    diagnostics = diagnostic_conditions(charter, suite_version)
    source_native_import_conditions = (
        V2_REQUIRED_SOURCE_NATIVE_IMPORT_CONDITIONS if suite_version == SUITE_V2 else []
    )
    plan = {
        "runs_per_task_condition": runs,
        "primary_conditions": conditions,
        "diagnostic_conditions": diagnostics,
        "source_native_import_conditions": source_native_import_conditions,
        "directions": {},
    }
    for direction, tasks in tasks_by_direction.items():
        non_ready_tasks = sorted(
            task_diagnostic_id(task, index)
            for index, task in enumerate(tasks)
            if task.get("status") != "ready"
            or not isinstance(task.get("id"), str)
            or not task.get("id")
        )
        plan["directions"][direction] = {
            "tasks": len(tasks),
            "ready": sum(1 for t in tasks if t.get("status") == "ready"),
            "skeleton_todo": sum(1 for t in tasks if t.get("status") == "skeleton_todo"),
            "planned_primary_runs": len(tasks) * len(conditions) * runs,
            "planned_source_native_import_runs": len(tasks)
            * len(source_native_import_conditions)
            * runs,
            "task_definitions_ready": not non_ready_tasks,
            "executable_ready": False,
            "non_ready_task_ids": non_ready_tasks,
        }
    plan["planned_primary_runs_total"] = sum(
        direction_plan["planned_primary_runs"]
        for direction_plan in plan["directions"].values()
    )
    plan["planned_source_native_import_runs_total"] = sum(
        direction_plan["planned_source_native_import_runs"]
        for direction_plan in plan["directions"].values()
    )
    plan["execution_scope"] = "plan_only"
    return plan


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--suite-version",
        choices=SUITE_CHOICES,
        default=SUITE_V1,
        help="report the legacy v1 plan or the v2 compatibility plan",
    )
    parser.add_argument(
        "--require-executable",
        action="store_true",
        help="fail unless every task is ready for executable live runs",
    )
    parser.add_argument("--artifacts", type=Path, help="directory of run artifacts to validate")
    parser.add_argument("--json-out", type=Path, help="write dry-run report JSON here")
    args = parser.parse_args(argv[1:])

    errors: list[str] = []
    charter = load_json(CHARTER)
    if charter.get("charter_version") != SUITE_V1:
        errors.append(f"charter: unexpected charter_version {charter.get('charter_version')!r}")
    tasks_by_direction = validate_tasks(charter, errors, args.suite_version)
    plan = build_plan(charter, tasks_by_direction, args.suite_version)

    if args.require_executable:
        errors.append(
            f"{args.suite_version}: executable runtime contract is unavailable; "
            "this command is offline plan-only"
        )

    artifact_count = 0
    if args.artifacts:
        artifact_count = validate_artifacts(
            args.artifacts,
            errors,
            args.suite_version,
            tasks_by_direction,
            charter["task_requirements"]["runs_per_task_condition"],
        )

    executable_ready = False
    task_definitions_ready = all(
        direction_plan["task_definitions_ready"]
        for direction_plan in plan["directions"].values()
    )
    charter_status = charter.get("status")
    compatibility_conversions = {}
    if args.suite_version == SUITE_V2:
        compatibility_conversions["conditions"] = V2_CONDITION_ALIASES

    report = {
        "suite": args.suite_version,
        "input_charter_version": charter.get("charter_version"),
        "mode": "offline_dry_run",
        "execution_scope": "plan_only",
        "charter_status": charter_status,
        "task_definitions_ready": task_definitions_ready,
        "executable_ready": executable_ready,
        "compatibility_conversions": compatibility_conversions,
        "schema_errors": errors,
        "artifacts_validated": artifact_count,
        "plan": plan,
        "passed": not errors,
    }
    if args.json_out:
        args.json_out.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")

    for err in errors:
        print(f"ERROR: {err}")
    print(json.dumps(report["plan"], indent=2))
    print(f"dry-run: {'PASS' if not errors else f'FAIL ({len(errors)} errors)'}")
    return 0 if not errors else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv))
