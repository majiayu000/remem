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
import schema_validate  # noqa: E402
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

    def _validate_artifact_json(self, artifact_json: str, suite: str) -> list[str]:
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "artifact.json"
            path.write_text(artifact_json, encoding="utf-8")
            errors: list[str] = []
            task_errors: list[str] = []
            tasks = run_dry.validate_tasks(self.charter, task_errors, suite)
            self.assertEqual(task_errors, [])
            self.assertEqual(
                run_dry.validate_artifacts(
                    Path(tmp),
                    errors,
                    suite,
                    tasks,
                    self.charter["task_requirements"]["runs_per_task_condition"],
                ),
                1,
            )
            return errors

    def _validate_artifact(self, artifact: object, suite: str) -> list[str]:
        return self._validate_artifact_json(json.dumps(artifact), suite)

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

    def test_v2_artifact_must_match_registered_task_tuple(self) -> None:
        unknown = {**self.v2_artifact, "task_id": "not-registered"}
        errors = self._validate_artifact(unknown, run_dry.SUITE_V2)
        self.assertTrue(any("is not registered in the task matrix" in error for error in errors))

        wrong_tuple = {
            **self.v2_artifact,
            "direction": "codex_to_claude",
            "source_host": "codex",
            "target_host": "claude_code",
        }
        errors = self._validate_artifact(wrong_tuple, run_dry.SUITE_V2)
        for field in ("direction", "source_host", "target_host"):
            self.assertTrue(any(f"artifact {field}" in error for error in errors))

        outside_run_range = {**self.v2_artifact, "run_index": 3}
        errors = self._validate_artifact(outside_run_range, run_dry.SUITE_V2)
        self.assertTrue(any("registered range 0..2" in error for error in errors))

    def test_artifact_json_rejects_duplicate_keys_and_non_finite_numbers(self) -> None:
        valid_json = json.dumps(self.v2_artifact)
        duplicate_suite = valid_json.replace(
            '"suite": "cross-host-v2"',
            '"suite": "cross-host-v1", "suite": "cross-host-v2"',
            1,
        )
        errors = self._validate_artifact_json(duplicate_suite, run_dry.SUITE_V2)
        self.assertTrue(any("duplicate object key 'suite'" in error for error in errors))

        non_finite = copy.deepcopy(self.v2_artifact)
        non_finite["metrics"]["handoff_fact_recall"] = float("nan")
        errors = self._validate_artifact_json(json.dumps(non_finite), run_dry.SUITE_V2)
        self.assertTrue(any("non-finite number 'NaN'" in error for error in errors))

        overflowing_float = valid_json.replace(
            '"artifacts": {',
            '"artifacts": {"overflow": 1e400, ',
            1,
        )
        errors = self._validate_artifact_json(overflowing_float, run_dry.SUITE_V2)
        self.assertTrue(any("non-finite number '1e400'" in error for error in errors))

    def test_standalone_validator_uses_the_same_strict_json_loader(self) -> None:
        valid_json = json.dumps(self.v2_artifact)
        duplicate_suite = valid_json.replace(
            '"suite": "cross-host-v2"',
            '"suite": "cross-host-v1", "suite": "cross-host-v2"',
            1,
        )
        with tempfile.TemporaryDirectory() as tmp:
            artifact_path = Path(tmp) / "artifact.json"
            artifact_path.write_text(duplicate_suite, encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "duplicate object key 'suite'"):
                schema_validate.validate_file(
                    artifact_path,
                    run_dry.RUN_SCHEMAS[run_dry.SUITE_V2],
                )

    def test_artifact_json_rejects_unsafe_integer_values(self) -> None:
        safe_boundary = copy.deepcopy(self.v2_artifact)
        safe_boundary["metrics"]["tokens_total"] = (1 << 53) - 1
        self.assertEqual(self._validate_artifact(safe_boundary, run_dry.SUITE_V2), [])

        unsafe_integer = json.dumps(self.v2_artifact).replace(
            '"tokens_total": 0',
            '"tokens_total": 9007199254740992',
            1,
        )
        errors = self._validate_artifact_json(unsafe_integer, run_dry.SUITE_V2)
        self.assertTrue(any("outside the safe JSON range" in error for error in errors))

    def test_v2_rejects_duplicate_artifact_tuple_identities(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            artifact_dir = Path(tmp)
            for name in ("first.json", "copied.json"):
                (artifact_dir / name).write_text(
                    json.dumps(self.v2_artifact),
                    encoding="utf-8",
                )
            task_errors: list[str] = []
            tasks = run_dry.validate_tasks(self.charter, task_errors, run_dry.SUITE_V2)
            self.assertEqual(task_errors, [])
            errors: list[str] = []
            self.assertEqual(
                run_dry.validate_artifacts(
                    artifact_dir,
                    errors,
                    run_dry.SUITE_V2,
                    tasks,
                    self.charter["task_requirements"]["runs_per_task_condition"],
                ),
                2,
            )
        self.assertEqual(sum("duplicate artifact tuple" in error for error in errors), 1)

    def test_v2_isolation_false_constant_requires_a_boolean(self) -> None:
        artifact = copy.deepcopy(self.v2_artifact)
        artifact["handoff_isolation"]["source_session_store_readable_by_target"] = 0
        errors = self._validate_artifact(artifact, run_dry.SUITE_V2)
        self.assertTrue(
            any(
                "source_session_store_readable_by_target" in error
                and "expected type ['boolean']" in error
                for error in errors
            )
        )

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
