import contextlib
import io
import json
import re
import sys
import tempfile
import unittest
from unittest import mock
from pathlib import Path

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
        self.calls: list[tuple[str, list[str], object]] = []

        def fake_run(name: str, command: list[str], **kwargs: object) -> check_pr_preflight.StepResult:
            commands.append(command)
            self.calls.append((name, command, kwargs.get("cwd", check_pr_preflight.ROOT)))
            return check_pr_preflight.StepResult(name, "PASS")

        def fake_expected_failure(
            name: str,
            command: list[str],
            expected_text: str,
            log_path: object,
            **kwargs: object,
        ) -> check_pr_preflight.StepResult:
            commands.append(command)
            self.calls.append((name, command, kwargs.get("cwd", check_pr_preflight.ROOT)))
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
            mock.patch("check_pr_preflight.shutil.copytree"),
        ):
            self.assertEqual(check_pr_preflight.main(), 0)
        return commands

    def test_all_three_eval_gates_use_generated_ignored_workspace(self) -> None:
        self.run_main()

        generators = [call for call in self.calls if call[1][:5] == ["cargo", "run", "--locked", "--", "bench"]]
        gates = [call for call in self.calls if call[1][:4] == ["cargo", "run", "--", "eval-gates"]]
        self.assertEqual(len(generators), 1)
        self.assertEqual(len(gates), 3)
        workspace = generators[0][2]
        self.assertTrue(workspace.is_relative_to(check_pr_preflight.ROOT / "target"))
        self.assertTrue(all(call[2] == workspace for call in gates))
        other_calls = [call for call in self.calls if call not in generators + gates]
        self.assertTrue(all(call[2] == check_pr_preflight.ROOT for call in other_calls))

    def test_native_security_generators_replace_actively_registered_reports(self) -> None:
        public_root = check_pr_preflight.ROOT / "eval/public"
        registered: dict[str, set[str]] = {}
        for path in (public_root / "memory/manifests").glob("adversarial-policy-v2*.json"):
            manifest = json.loads(path.read_text(encoding="utf-8"))
            for report_path in manifest["reports"]:
                report = json.loads((public_root / report_path).read_text(encoding="utf-8"))
                registered[f"eval/public/{report_path}"] = {
                    run_path.rsplit("/", 2)[0] for run_path in report["run_artifacts"]
                }
        self.assertEqual(len(registered), 4)
        for os_name, arch in [
            ("darwin", "arm64"), ("darwin", "x86_64"),
            ("linux", "aarch64"), ("linux", "x86_64"),
        ]:
            with (
                self.subTest(os=os_name, arch=arch),
                mock.patch.object(sys, "platform", os_name),
                mock.patch("check_pr_preflight.platform.machine", return_value=arch),
                contextlib.redirect_stdout(io.StringIO()),
            ):
                commands = self.run_main()
                generator = next(command for command in commands
                                 if command[:5] == ["cargo", "run", "--locked", "--", "bench"])
                report_path = generator[generator.index("--json-out") + 1]
                artifact_prefix = generator[generator.index("--artifact-prefix") + 1]
                self.assertIn(report_path, registered)
                self.assertEqual(registered[report_path], {artifact_prefix})

        workflow = (check_pr_preflight.ROOT / ".github/workflows/ci.yml").read_text(encoding="utf-8")
        generator = workflow.split(
            "- name: Generate current security evidence in ignored eval workspace", 1
        )[1].split("\n      - name:", 1)[0]
        report = re.search(r"--json-out\s+(\S+)", generator)
        artifacts = re.search(r"--artifact-prefix\s+(\S+)", generator)
        self.assertIsNotNone(report)
        self.assertIsNotNone(artifacts)
        assert report is not None and artifacts is not None
        self.assertIn(report[1], registered)
        self.assertEqual(registered[report[1]], {artifacts[1]})

    def test_benchmark_failure_prevents_using_stale_evidence(self) -> None:
        commands: list[list[str]] = []
        def fake_run(name: str, command: list[str], **kwargs: object) -> check_pr_preflight.StepResult:
            commands.append(command)
            status = "FAIL" if command[:5] == ["cargo", "run", "--locked", "--", "bench"] else "PASS"
            return check_pr_preflight.StepResult(name, status)
        with (
            mock.patch.object(sys, "argv", ["check_pr_preflight.py"]),
            mock.patch.object(check_pr_preflight, "run", side_effect=fake_run),
            mock.patch.object(check_pr_preflight, "run_expected_failure") as negative,
            mock.patch.object(check_pr_preflight, "add_pr_body_steps"),
            mock.patch("check_pr_preflight.shutil.copytree"),
            contextlib.redirect_stdout(io.StringIO()),
        ):
            self.assertEqual(check_pr_preflight.main(), 1)
        self.assertFalse(any(command[:4] == ["cargo", "run", "--", "eval-gates"] for command in commands))
        self.assertTrue(any(command[:2] == ["cargo", "test"] for command in commands))
        negative.assert_not_called()

    def test_missing_repository_target_is_created_for_custom_cargo_target(self) -> None:
        with tempfile.TemporaryDirectory() as raw_tmp:
            root = Path(raw_tmp)
            with (
                mock.patch.object(check_pr_preflight, "ROOT", root),
                mock.patch.dict("os.environ", {"CARGO_TARGET_DIR": str(root / "elsewhere")}),
                contextlib.redirect_stdout(io.StringIO()),
            ):
                self.run_main()
            self.assertTrue((root / "target").is_dir())

    def test_unknown_platform_reports_failure_and_runs_independent_tests(self) -> None:
        commands: list[list[str]] = []
        output = io.StringIO()
        def fake_run(name: str, command: list[str], **kwargs: object) -> check_pr_preflight.StepResult:
            commands.append(command)
            return check_pr_preflight.StepResult(name, "PASS")
        with (
            mock.patch.object(sys, "argv", ["check_pr_preflight.py"]),
            mock.patch.object(sys, "platform", "linux"),
            mock.patch("check_pr_preflight.platform.machine", return_value="riscv64"),
            mock.patch.object(check_pr_preflight, "run", side_effect=fake_run),
            mock.patch.object(check_pr_preflight, "run_expected_failure") as negative,
            mock.patch.object(check_pr_preflight, "add_pr_body_steps"),
            mock.patch("check_pr_preflight.shutil.copytree"),
            contextlib.redirect_stdout(output),
        ):
            self.assertEqual(check_pr_preflight.main(), 1)
        self.assertIn("unsupported platform: linux/riscv64", output.getvalue())
        self.assertTrue(any(command[:2] == ["cargo", "test"] for command in commands))
        self.assertFalse(any(command[:5] == ["cargo", "run", "--locked", "--", "bench"] for command in commands))
        negative.assert_not_called()

    def test_default_command_caps_rust_test_harness_at_four_threads(self) -> None:
        commands = self.run_main()

        self.assertEqual(
            commands[-1],
            [
                "cargo",
                "test",
                "--no-default-features",
                "--features",
                "local-onnx",
                "--",
                "--test-threads",
                "4",
            ],
        )

    def test_override_changes_rust_test_harness_thread_count(self) -> None:
        commands = self.run_main("--cargo-test-threads", "8")

        self.assertEqual(
            commands[-1],
            [
                "cargo",
                "test",
                "--no-default-features",
                "--features",
                "local-onnx",
                "--",
                "--test-threads",
                "8",
            ],
        )

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

    def test_eval_e2e_target_requires_eval_feature(self) -> None:
        source = (check_pr_preflight.ROOT / "tests/e2e_eval.rs").read_text(
            encoding="utf-8"
        )

        self.assertIn('#![cfg(feature = "eval")]', source)

    def test_ci_eval_phase_runs_eval_e2e_target(self) -> None:
        workflow = (check_pr_preflight.ROOT / ".github/workflows/ci.yml").read_text(
            encoding="utf-8"
        )

        self.assertIn(
            "cargo test --features eval --lib eval --test e2e_eval", workflow
        )

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
