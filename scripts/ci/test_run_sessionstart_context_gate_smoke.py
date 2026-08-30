import contextlib
import io
import json
import os
import stat
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest import mock

import run_sessionstart_context_gate_smoke


class SessionStartSmokeRunnerTests(unittest.TestCase):
    def test_parser_uses_cargo_reported_cross_target_executable(self) -> None:
        executable = "/tmp/custom-target/aarch64-unknown-linux-gnu/debug/remem"
        output = "\n".join(
            [
                json.dumps(
                    {
                        "reason": "compiler-artifact",
                        "target": {"name": "dependency", "kind": ["lib"]},
                        "executable": None,
                    }
                ),
                json.dumps(
                    {
                        "reason": "compiler-artifact",
                        "target": {"name": "remem", "kind": ["bin"]},
                        "executable": executable,
                    }
                ),
            ]
        )

        self.assertEqual(
            run_sessionstart_context_gate_smoke.parse_remem_executable(output),
            Path(executable),
        )

    def test_main_passes_reported_artifact_to_fixture_under_hostile_parent_env(self) -> None:
        with tempfile.TemporaryDirectory(prefix="remem-smoke-runner-") as raw_tmp:
            executable = Path(raw_tmp) / "custom-target/triple/debug/remem"
            executable.parent.mkdir(parents=True)
            executable.write_text("#!/bin/sh\nexit 0\n", encoding="utf-8")
            executable.chmod(executable.stat().st_mode | stat.S_IXUSR)
            cargo_output = json.dumps(
                {
                    "reason": "compiler-artifact",
                    "target": {"name": "remem", "kind": ["bin"]},
                    "executable": str(executable),
                }
            )
            completed = [
                subprocess.CompletedProcess([], 0, cargo_output, ""),
                subprocess.CompletedProcess([], 0, "", ""),
            ]

            with mock.patch.object(
                run_sessionstart_context_gate_smoke.subprocess,
                "run",
                side_effect=completed,
            ) as run:
                self.assertEqual(run_sessionstart_context_gate_smoke.main(), 0)

            build_call, smoke_call = run.call_args_list
            self.assertIn("--message-format=json", build_call.args[0])
            self.assertEqual(
                smoke_call.args[0],
                [
                    str(
                        run_sessionstart_context_gate_smoke.ROOT
                        / "scripts/ci/smoke_sessionstart_context_gate.sh"
                    ),
                    str(executable),
                ],
            )
            smoke_env = smoke_call.kwargs["env"]
            self.assertEqual(smoke_env["HOME"], "/nonexistent/remem-smoke-parent-home")
            self.assertEqual(
                smoke_env["REMEM_UNDECLARED_PARENT_SENTINEL"], "hostile"
            )
            self.assertEqual(smoke_call.kwargs["cwd"], run_sessionstart_context_gate_smoke.ROOT)

    def test_cargo_failure_is_reported_without_running_fixture(self) -> None:
        failure = subprocess.CompletedProcess([], 101, "", "cargo failed\n")
        with (
            mock.patch.object(
                run_sessionstart_context_gate_smoke.subprocess,
                "run",
                return_value=failure,
            ) as run,
            mock.patch.dict(os.environ, {}, clear=False),
        ):
            self.assertEqual(run_sessionstart_context_gate_smoke.main(), 1)

        run.assert_called_once()

    def test_missing_cargo_reports_clear_error(self) -> None:
        stderr = io.StringIO()
        with (
            mock.patch.object(
                run_sessionstart_context_gate_smoke.subprocess,
                "run",
                side_effect=FileNotFoundError(2, "No such file or directory", "cargo"),
            ),
            contextlib.redirect_stderr(stderr),
        ):
            self.assertEqual(run_sessionstart_context_gate_smoke.main(), 1)

        self.assertIn("error: failed to start Cargo", stderr.getvalue())
        self.assertIn("cargo", stderr.getvalue())

    def test_fixture_spawn_error_is_reported(self) -> None:
        with tempfile.TemporaryDirectory(prefix="remem-smoke-runner-") as raw_tmp:
            executable = Path(raw_tmp) / "remem"
            executable.write_text("#!/bin/sh\nexit 0\n", encoding="utf-8")
            executable.chmod(executable.stat().st_mode | stat.S_IXUSR)
            cargo_output = json.dumps(
                {
                    "reason": "compiler-artifact",
                    "target": {"name": "remem", "kind": ["bin"]},
                    "executable": str(executable),
                }
            )
            stderr = io.StringIO()
            with (
                mock.patch.object(
                    run_sessionstart_context_gate_smoke.subprocess,
                    "run",
                    side_effect=[
                        subprocess.CompletedProcess([], 0, cargo_output, ""),
                        PermissionError(13, "Permission denied", "smoke fixture"),
                    ],
                ),
                contextlib.redirect_stderr(stderr),
            ):
                self.assertEqual(run_sessionstart_context_gate_smoke.main(), 1)

        self.assertIn("error: failed to start SessionStart smoke fixture", stderr.getvalue())
        self.assertIn("smoke fixture", stderr.getvalue())


if __name__ == "__main__":
    unittest.main()
