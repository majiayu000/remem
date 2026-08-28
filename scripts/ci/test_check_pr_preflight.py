import contextlib
import io
import os
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

import check_pr_preflight


EXPECTED_SESSIONSTART_BUILD_COMMAND = ["cargo", "build", "--locked", "--bin", "remem"]


def expected_sessionstart_smoke_command() -> list[str]:
    root = Path(__file__).resolve().parents[2]
    configured_target = Path(os.environ.get("CARGO_TARGET_DIR", "target"))
    target = configured_target if configured_target.is_absolute() else root / configured_target
    binary = str((target / "debug/remem").resolve())
    return [
        "env",
        "HOME=/nonexistent/remem-smoke-parent-home",
        "XDG_CONFIG_HOME=/nonexistent/remem-smoke-parent-xdg-config",
        "XDG_DATA_HOME=/nonexistent/remem-smoke-parent-xdg-data",
        "REMEM_CONTEXT_HOST=claude-code",
        "REMEM_CONTEXT_GATE=off",
        "REMEM_CONTEXT_GATE_HOSTS=claude-code",
        "REMEM_CONTEXT_DEBUG=1",
        "REMEM_CONTEXT_GATE_RETENTION_DAYS=0",
        "REMEM_CONTEXT_BUNDLE_RENDER_MODE=invalid",
        "REMEM_CONTEXT_TOTAL_CHAR_LIMIT=invalid",
        "REMEM_UNDECLARED_PARENT_SENTINEL=hostile",
        "scripts/ci/smoke_sessionstart_context_gate.sh",
        binary,
    ]


def assert_sessionstart_smoke_registration(commands: list[list[str]]) -> None:
    build_indexes = [
        index
        for index, command in enumerate(commands)
        if command == EXPECTED_SESSIONSTART_BUILD_COMMAND
    ]
    smoke_indexes = [
        index
        for index, command in enumerate(commands)
        if command == expected_sessionstart_smoke_command()
    ]
    if len(build_indexes) != 1 or len(smoke_indexes) != 1:
        raise AssertionError(
            "preflight must execute independent build and hostile-environment smoke argv once"
        )
    if build_indexes[0] >= smoke_indexes[0]:
        raise AssertionError("preflight must build remem before SessionStart smoke")


class PreflightCargoTestThreadsTests(unittest.TestCase):
    def run_main(self, *arguments: str) -> list[list[str]]:
        commands: list[list[str]] = []

        def fake_run(name: str, command: list[str], **_: object) -> check_pr_preflight.StepResult:
            commands.append(command)
            return check_pr_preflight.StepResult(name, "PASS")

        def fake_expected_failure(
            name: str,
            command: list[str],
            expected_text: str,
            log_path: object,
        ) -> check_pr_preflight.StepResult:
            commands.append(command)
            return check_pr_preflight.StepResult(name, "PASS")

        with (
            mock.patch.object(sys, "argv", ["check_pr_preflight.py", *arguments]),
            mock.patch.object(check_pr_preflight, "run", side_effect=fake_run),
            mock.patch.object(
                check_pr_preflight,
                "run_expected_failure",
                side_effect=fake_expected_failure,
            ),
            mock.patch.object(check_pr_preflight, "add_pr_body_steps"),
        ):
            self.assertEqual(check_pr_preflight.main(), 0)
        return commands

    def test_default_command_caps_rust_test_harness_at_four_threads(self) -> None:
        commands = self.run_main()

        self.assertEqual(commands[-1], ["cargo", "test", "--", "--test-threads", "4"])

    def test_override_changes_rust_test_harness_thread_count(self) -> None:
        commands = self.run_main("--cargo-test-threads", "8")

        self.assertEqual(commands[-1], ["cargo", "test", "--", "--test-threads", "8"])

    def test_zero_and_negative_thread_counts_are_rejected_before_gates(self) -> None:
        for value in ("0", "-1"):
            with self.subTest(value=value):
                stderr = io.StringIO()
                with (
                    mock.patch.object(
                        sys,
                        "argv",
                        ["check_pr_preflight.py", "--cargo-test-threads", value],
                    ),
                    contextlib.redirect_stderr(stderr),
                    mock.patch.object(
                        check_pr_preflight,
                        "fast_steps",
                        side_effect=AssertionError("gates must not run"),
                    ),
                ):
                    with self.assertRaises(SystemExit) as raised:
                        check_pr_preflight.main()
                self.assertEqual(raised.exception.code, 2)
                self.assertIn("must be a positive integer", stderr.getvalue())

    def test_fast_mode_omits_cargo_test(self) -> None:
        commands = self.run_main("--fast")

        self.assertFalse(any(command[:2] == ["cargo", "test"] for command in commands))

    def test_fast_mode_runs_surface_lifecycle_check_and_self_test(self) -> None:
        commands = self.run_main("--fast")

        self.assertIn(
            ["python3", "scripts/ci/check_documentation_contracts.py"], commands
        )
        self.assertIn(
            ["python3", "scripts/ci/test_check_documentation_contracts.py"], commands
        )
        assert_sessionstart_smoke_registration(commands)
        self.assertIn(["python3", "scripts/ci/check_public_surface.py"], commands)
        self.assertIn(
            ["python3", "scripts/ci/check_surface_baseline.py", "origin/main"],
            commands,
        )
        self.assertIn(
            ["python3", "scripts/ci/check_public_surface.py", "--self-test"],
            commands,
        )
        self.assertIn(["python3", "scripts/ci/surface_lifecycle_rest.py"], commands)

    def test_full_mode_runs_sessionstart_smoke_once(self) -> None:
        commands = self.run_main()

        assert_sessionstart_smoke_registration(commands)

    def test_noop_sessionstart_command_fails_independent_registration(self) -> None:
        with mock.patch.object(
            check_pr_preflight,
            "sessionstart_smoke_command",
            return_value=["true"],
            create=True,
        ):
            commands = self.run_main("--fast")

        with self.assertRaisesRegex(AssertionError, "independent build"):
            assert_sessionstart_smoke_registration(commands)

    def test_noop_sessionstart_build_fails_independent_registration(self) -> None:
        with mock.patch.object(
            check_pr_preflight,
            "SESSIONSTART_SMOKE_BUILD_COMMAND",
            ["true"],
            create=True,
        ):
            commands = self.run_main("--fast")

        with self.assertRaisesRegex(AssertionError, "independent build"):
            assert_sessionstart_smoke_registration(commands)

    def test_relative_cargo_target_dir_resolves_from_repository_root(self) -> None:
        with mock.patch.dict(os.environ, {"CARGO_TARGET_DIR": "build/smoke-target"}):
            commands = self.run_main("--fast")
            assert_sessionstart_smoke_registration(commands)

    def test_absolute_cargo_target_dir_is_preserved(self) -> None:
        with tempfile.TemporaryDirectory(prefix="remem-target-") as target:
            with mock.patch.dict(os.environ, {"CARGO_TARGET_DIR": target}):
                commands = self.run_main("--fast")
                assert_sessionstart_smoke_registration(commands)


if __name__ == "__main__":
    unittest.main()
