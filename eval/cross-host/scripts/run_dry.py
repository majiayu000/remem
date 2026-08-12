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
from schema_validate import validate  # noqa: E402

SUITE_ROOT = Path(__file__).resolve().parent.parent
TASK_SCHEMA = SUITE_ROOT / "schemas" / "cross-host-task.schema.json"
RUN_SCHEMA = SUITE_ROOT / "schemas" / "cross-host-run.schema.json"
CHARTER = SUITE_ROOT / "benchmark-charter.json"
SUITE_V1 = "cross-host-v1"
SUITE_V2 = "cross-host-v2"
SUITE_CHOICES = (SUITE_V1, SUITE_V2)
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


def load_json(path: Path) -> dict:
    return json.loads(path.read_text(encoding="utf-8"))


def normalize_condition(condition: str, suite_version: str) -> str:
    if suite_version == SUITE_V2:
        return V2_CONDITION_ALIASES.get(condition, condition)
    return condition


def primary_conditions(charter: dict, suite_version: str) -> list[str]:
    return [
        normalize_condition(condition, suite_version)
        for condition in charter["primary_conditions"]
    ]


def diagnostic_conditions(charter: dict, suite_version: str) -> list[str]:
    conditions = [
        normalize_condition(condition, suite_version)
        for condition in charter.get("diagnostic_conditions", [])
    ]
    if suite_version == SUITE_V2:
        for condition in V2_REQUIRED_SOURCE_NATIVE_IMPORT_CONDITIONS:
            if condition not in conditions:
                conditions.append(condition)
    return conditions


def validate_tasks(charter: dict, errors: list[str]) -> dict[str, list[dict]]:
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
            if task.get("id") in seen_ids:
                errors.append(f"{rel}: duplicate task id {task.get('id')!r}")
            seen_ids.add(task.get("id", ""))
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

        if len(tasks) < min_tasks:
            errors.append(f"{direction}: {len(tasks)} tasks, charter requires {min_tasks}")
        covered = {t.get("category") for t in tasks}
        for missing in sorted(required_categories - covered):
            errors.append(f"{direction}: missing required category {missing!r}")
    return tasks_by_direction


def validate_artifacts(artifact_dir: Path, errors: list[str], suite_version: str) -> int:
    run_schema = load_json(RUN_SCHEMA)
    count = 0
    for path in sorted(artifact_dir.rglob("*.json")):
        count += 1
        artifact = load_json(path)
        for err in validate(artifact, run_schema):
            errors.append(f"{path}: {err}")
        condition = artifact.get("condition")
        if (
            suite_version == SUITE_V2
            and artifact.get("suite") == SUITE_V2
            and condition == "remem_shared"
        ):
            errors.append(f"{path}: cross-host-v2 artifacts must use remem_shared_startup")
        normalized_condition = normalize_condition(condition, suite_version)
        attribution = artifact.get("attribution", {})
        if normalized_condition == "no_memory":
            for key in ("promoted_memory_refs", "selected_context_item_refs", "used_refs"):
                if attribution.get(key):
                    errors.append(f"{path}: no_memory run must have empty attribution.{key}")
        if normalized_condition == "remem_shared_startup":
            for key in ("source_session_refs", "captured_event_refs", "promoted_memory_refs"):
                if not attribution.get(key):
                    errors.append(f"{path}: remem_shared_startup run must record attribution.{key}")
        if not artifact.get("handoff_isolation", {}).get("leak_scan_passed", False):
            errors.append(f"{path}: leak_scan_passed must be true for a counted run")
    return count


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
        skeleton_tasks = sorted(t["id"] for t in tasks if t.get("status") != "ready")
        plan["directions"][direction] = {
            "tasks": len(tasks),
            "ready": sum(1 for t in tasks if t.get("status") == "ready"),
            "skeleton_todo": sum(1 for t in tasks if t.get("status") == "skeleton_todo"),
            "planned_primary_runs": len(tasks) * len(conditions) * runs,
            "planned_source_native_import_runs": len(tasks)
            * len(source_native_import_conditions)
            * runs,
            "executable_ready": not skeleton_tasks,
            "non_ready_task_ids": skeleton_tasks,
        }
    plan["planned_primary_runs_total"] = sum(
        direction_plan["planned_primary_runs"]
        for direction_plan in plan["directions"].values()
    )
    plan["planned_source_native_import_runs_total"] = sum(
        direction_plan["planned_source_native_import_runs"]
        for direction_plan in plan["directions"].values()
    )
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
    tasks_by_direction = validate_tasks(charter, errors)
    plan = build_plan(charter, tasks_by_direction, args.suite_version)

    if args.require_executable:
        for direction, direction_plan in plan["directions"].items():
            for task_id in direction_plan["non_ready_task_ids"]:
                errors.append(f"{direction}: task {task_id!r} is not executable-ready")

    artifact_count = 0
    if args.artifacts:
        artifact_count = validate_artifacts(args.artifacts, errors, args.suite_version)

    executable_ready = all(
        direction_plan["executable_ready"] for direction_plan in plan["directions"].values()
    )
    charter_status = charter.get("status")
    compatibility_conversions = {}
    if args.suite_version == SUITE_V2:
        compatibility_conversions["conditions"] = V2_CONDITION_ALIASES
        charter_status = (
            "executable_no_runs" if executable_ready else "infrastructure_only_no_runs"
        )

    report = {
        "suite": args.suite_version,
        "input_charter_version": charter.get("charter_version"),
        "mode": "offline_dry_run",
        "charter_status": charter_status,
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
