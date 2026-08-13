#!/usr/bin/env python3
"""Discriminating regression tests for the cross-host offline dry-run."""

from __future__ import annotations

import copy
import io
import json
import shutil
import sys
import tempfile
import unittest
from contextlib import redirect_stdout
from pathlib import Path
from unittest import mock

SCRIPT_DIR = Path(__file__).resolve().parent
sys.path.insert(0, str(SCRIPT_DIR))

import run_dry  # noqa: E402
from schema_validate import validate  # noqa: E402


class RunDryTests(unittest.TestCase):
    def setUp(self) -> None:
        self.charter = run_dry.load_json(run_dry.CHARTER)
        self.v1_artifact = run_dry.load_json(
            run_dry.SUITE_ROOT / "examples" / "run-artifact-valid.json"
        )
        self.v2_artifact = run_dry.load_json(
            run_dry.SUITE_ROOT / "examples" / "run-artifact-v2-plan-valid.json"
        )

    def _validate_artifact(self, artifact: object, suite: str) -> list[str]:
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "artifact.json"
            path.write_text(json.dumps(artifact), encoding="utf-8")
            errors: list[str] = []
            self.assertEqual(run_dry.validate_artifacts(Path(tmp), errors, suite), 1)
            return errors

    def _isolated_task_dirs(self, root: Path) -> dict[str, Path]:
        mapping = {
            "claude_to_codex": root / "claude-to-codex",
            "codex_to_claude": root / "codex-to-claude",
        }
        for direction, source in run_dry.TASK_DIRS.items():
            shutil.copytree(source, mapping[direction])
        return mapping

    def test_run_schemas_are_strictly_version_isolated(self) -> None:
        v1_schema = run_dry.load_json(run_dry.RUN_SCHEMAS[run_dry.SUITE_V1])
        v2_schema = run_dry.load_json(run_dry.RUN_SCHEMAS[run_dry.SUITE_V2])

        self.assertEqual(validate(self.v1_artifact, v1_schema), [])
        self.assertEqual(validate(self.v2_artifact, v2_schema), [])
        self.assertTrue(validate(self.v1_artifact, v2_schema))
        self.assertTrue(validate(self.v2_artifact, v1_schema))

        v1_with_v2_condition = {**self.v1_artifact, "condition": "remem_shared_startup"}
        v2_with_v1_condition = {**self.v2_artifact, "condition": "remem_shared"}
        self.assertTrue(validate(v1_with_v2_condition, v1_schema))
        self.assertTrue(validate(v2_with_v1_condition, v2_schema))

    def test_requested_suite_must_match_artifact_suite(self) -> None:
        errors = self._validate_artifact(self.v1_artifact, run_dry.SUITE_V2)
        self.assertTrue(any("does not match requested suite 'cross-host-v2'" in e for e in errors))

    def test_v1_remem_attribution_does_not_fall_back(self) -> None:
        artifact = copy.deepcopy(self.v1_artifact)
        artifact["attribution"]["source_session_refs"] = []
        artifact["attribution"]["captured_event_refs"] = []
        artifact["attribution"]["promoted_memory_refs"] = []

        errors = self._validate_artifact(artifact, run_dry.SUITE_V1)
        self.assertEqual(sum("remem_shared run must record attribution" in e for e in errors), 3)

    def test_non_string_v2_condition_reports_schema_error(self) -> None:
        artifact = run_dry.load_json(
            run_dry.SUITE_ROOT / "examples" / "run-artifact-v2-invalid-condition.json"
        )
        errors = self._validate_artifact(artifact, run_dry.SUITE_V2)
        self.assertTrue(any("$.condition" in e and "enum" in e for e in errors))

    def test_missing_task_id_keeps_schema_diagnostics_and_plan(self) -> None:
        with tempfile.TemporaryDirectory(dir=run_dry.SUITE_ROOT) as tmp:
            task_dirs = self._isolated_task_dirs(Path(tmp))
            task_path = next(task_dirs["claude_to_codex"].glob("*.json"))
            task = run_dry.load_json(task_path)
            task.pop("id")
            task_path.write_text(json.dumps(task), encoding="utf-8")

            errors: list[str] = []
            with mock.patch.object(run_dry, "TASK_DIRS", task_dirs):
                tasks = run_dry.validate_tasks(self.charter, errors, run_dry.SUITE_V2)
            plan = run_dry.build_plan(self.charter, tasks, run_dry.SUITE_V2)

        self.assertTrue(any("missing required key 'id'" in error for error in errors))
        non_ready = plan["directions"]["claude_to_codex"]["non_ready_task_ids"]
        self.assertTrue(any(task_id.startswith("<missing-id:") for task_id in non_ready))

    def test_v2_rejects_an_extra_unique_task(self) -> None:
        with tempfile.TemporaryDirectory(dir=run_dry.SUITE_ROOT) as tmp:
            task_dirs = self._isolated_task_dirs(Path(tmp))
            source = next(task_dirs["claude_to_codex"].glob("*.json"))
            extra = run_dry.load_json(source)
            extra["id"] = "cc2cx-extra-task"
            (task_dirs["claude_to_codex"] / "cc2cx-extra-task.json").write_text(
                json.dumps(extra), encoding="utf-8"
            )

            errors: list[str] = []
            with mock.patch.object(run_dry, "TASK_DIRS", task_dirs):
                run_dry.validate_tasks(self.charter, errors, run_dry.SUITE_V2)

        self.assertTrue(any("found 13 tasks and 13 unique ids" in error for error in errors))

    def test_v2_is_plan_only_even_if_task_definitions_are_ready(self) -> None:
        ready_tasks = {
            direction: [{"id": f"{direction}-ready", "status": "ready"}]
            for direction in run_dry.TASK_DIRS
        }
        plan = run_dry.build_plan(self.charter, ready_tasks, run_dry.SUITE_V2)
        for direction in plan["directions"].values():
            self.assertTrue(direction["task_definitions_ready"])
            self.assertFalse(direction["executable_ready"])
        self.assertEqual(plan["execution_scope"], "plan_only")

    def test_v2_exact_plan_and_require_executable_fails(self) -> None:
        errors: list[str] = []
        tasks = run_dry.validate_tasks(self.charter, errors, run_dry.SUITE_V2)
        plan = run_dry.build_plan(self.charter, tasks, run_dry.SUITE_V2)
        self.assertEqual(errors, [])
        self.assertEqual(plan["planned_primary_runs_total"], 288)
        self.assertEqual(plan["planned_source_native_import_runs_total"], 144)

        with tempfile.TemporaryDirectory() as tmp, redirect_stdout(io.StringIO()):
            report_path = Path(tmp) / "report.json"
            exit_code = run_dry.main(
                [
                    "run_dry.py",
                    "--suite-version",
                    run_dry.SUITE_V2,
                    "--require-executable",
                    "--json-out",
                    str(report_path),
                ]
            )
            report = run_dry.load_json(report_path)
        self.assertEqual(exit_code, 1)
        self.assertFalse(report["executable_ready"])
        self.assertTrue(any("offline plan-only" in error for error in report["schema_errors"]))


if __name__ == "__main__":
    unittest.main()
