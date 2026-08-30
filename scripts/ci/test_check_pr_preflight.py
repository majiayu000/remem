import contextlib
import io
import sys
import unittest
from unittest import mock

import check_pr_preflight


EXPECTED_SESSIONSTART_SMOKE_COMMAND = [
    "python3",
    "scripts/ci/run_sessionstart_context_gate_smoke.py",
]
EXPECTED_SESSIONSTART_RUNNER_TEST_COMMAND = [
    "python3",
    "scripts/ci/test_run_sessionstart_context_gate_smoke.py",
]


def assert_sessionstart_smoke_registration(commands: list[list[str]]) -> None:
    matches = [
        command for command in commands if command == EXPECTED_SESSIONSTART_SMOKE_COMMAND
    ]
    if len(matches) != 1:
        raise AssertionError("preflight must execute the artifact-resolving smoke once")
    test_matches = [
        command
        for command in commands
        if command == EXPECTED_SESSIONSTART_RUNNER_TEST_COMMAND
    ]
    if len(test_matches) != 1:
        raise AssertionError("preflight must execute the SessionStart runner tests once")


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
            "SESSIONSTART_SMOKE_COMMAND",
            ["true"],
            create=True,
        ):
            commands = self.run_main("--fast")

        with self.assertRaisesRegex(AssertionError, "artifact-resolving smoke"):
            assert_sessionstart_smoke_registration(commands)

    def test_noop_sessionstart_runner_tests_fail_independent_registration(self) -> None:
        with mock.patch.object(
            check_pr_preflight,
            "SESSIONSTART_RUNNER_TEST_COMMAND",
            ["true"],
            create=True,
        ):
            commands = self.run_main("--fast")

        with self.assertRaisesRegex(AssertionError, "runner tests"):
            assert_sessionstart_smoke_registration(commands)

    def test_preflight_does_not_construct_a_fixed_cargo_artifact_path(self) -> None:
        commands = self.run_main("--fast")

        assert_sessionstart_smoke_registration(commands)
        self.assertFalse(
            any("target/debug/remem" in argument for command in commands for argument in command)
        )


if __name__ == "__main__":
    unittest.main()
