import os
import re
import subprocess
import textwrap
import unittest
from dataclasses import dataclass, field
from pathlib import Path


EXPECTED_WORKFLOW_SMOKE_COMMAND = "python3 scripts/ci/run_sessionstart_context_gate_smoke.py"
EXPECTED_WORKFLOW_RUNNER_TEST_COMMAND = "python3 scripts/ci/test_run_sessionstart_context_gate_smoke.py"
REQUIRED_CI_JOBS = {
    "runtime_checks",
    "benchmark_evidence",
    "windows_local_embedding_security",
    "windows_runtime_regressions",
}
SAFE_SHELLS = {"", "bash"}
SAFE_WORKING_DIRECTORIES = {"", ".", "${{ github.workspace }}"}


@dataclass
class WorkflowJob:
    job_id: str = ""
    source: str = ""
    fields: dict[str, str] = field(default_factory=dict)
    inherited_execution_fields: dict[str, str] = field(default_factory=dict)
    steps: list[dict[str, str]] = field(default_factory=list)


def yaml_scalar(raw: str) -> str:
    value = raw.strip()
    if len(value) >= 2 and value[0] == value[-1] and value[0] in {'"', "'"}:
        return value[1:-1]
    return value


def workflow_jobs(text: str) -> list[WorkflowJob]:
    """Narrowly parse job and step execution fields without production constants."""
    jobs: list[WorkflowJob] = []
    current_job: WorkflowJob | None = None
    current_step: dict[str, str] | None = None
    in_jobs = False
    for line in text.splitlines():
        if line == "jobs:":
            in_jobs = True
            continue
        if not in_jobs:
            continue
        job_match = re.fullmatch(r"  ([A-Za-z0-9_-]+):\s*", line)
        if job_match:
            current_job = WorkflowJob(job_id=job_match.group(1), source=line + "\n")
            jobs.append(current_job)
            current_step = None
            continue
        if current_job is None:
            continue
        current_job.source += line + "\n"
        step_match = re.match(r"^      -\s+(.+)$", line)
        if step_match:
            current_step = {}
            current_job.steps.append(current_step)
            field_text = step_match.group(1)
            if ":" in field_text:
                key, value = field_text.split(":", maxsplit=1)
                current_step[key.strip()] = yaml_scalar(value)
            continue
        job_field = re.match(r"^    ([A-Za-z0-9_-]+):\s*(.*)$", line)
        if job_field:
            current_step = None
            current_job.fields[job_field.group(1)] = yaml_scalar(job_field.group(2))
            continue
        step_field = re.match(r"^        ([A-Za-z0-9_-]+):\s*(.*)$", line)
        if current_step is not None and step_field:
            current_step[step_field.group(1)] = yaml_scalar(step_field.group(2))
            continue
        inherited = re.match(
            r"^\s{6,}((?:shell|working-directory)):\s*(.*)$", line
        )
        if current_step is None and inherited:
            current_job.inherited_execution_fields[inherited.group(1)] = yaml_scalar(
                inherited.group(2)
            )
    return jobs


def workflow_job_with_command(text: str, command: str) -> WorkflowJob:
    return next(
        job
        for job in workflow_jobs(text)
        if any(step.get("run") == command for step in job.steps)
    )


def execution_violations(
    label: str,
    fields: dict[str, str],
    inherited: dict[str, str],
) -> list[str]:
    violations: list[str] = []
    if "if" in fields:
        violations.append(f"{label} must be unconditional")
    if fields.get("continue-on-error", "").lower() not in {"", "false"}:
        violations.append(f"{label} must fail CI on error")
    shell = fields.get("shell", inherited.get("shell", ""))
    if shell not in SAFE_SHELLS:
        violations.append(f"{label} must use the default or standard bash shell")
    working_directory = fields.get(
        "working-directory", inherited.get("working-directory", "")
    )
    if working_directory not in SAFE_WORKING_DIRECTORIES:
        violations.append(f"{label} must run from the repository root")
    if "timeout-minutes" in fields:
        violations.append(f"{label} must not be disabled by a local timeout")
    return violations


def workflow_smoke_registration_violations(text: str) -> list[str]:
    """Independently enforce an executable build followed by an isolated smoke."""
    matches: list[tuple[WorkflowJob, int, dict[str, str]]] = []
    for job in workflow_jobs(text):
        for index, step in enumerate(job.steps):
            if step.get("run") == EXPECTED_WORKFLOW_SMOKE_COMMAND:
                matches.append((job, index, step))
    violations: list[str] = []
    if len(matches) != 1:
        violations.append("CI must execute the exact SessionStart smoke command once")
    if len(matches) == 1:
        smoke_job, _, smoke_step = matches[0]
        violations.extend(execution_violations("SessionStart smoke job", smoke_job.fields, {}))
        violations.extend(
            execution_violations(
                "SessionStart smoke step",
                smoke_step,
                smoke_job.inherited_execution_fields,
            )
        )
    if text.count(EXPECTED_WORKFLOW_SMOKE_COMMAND) != 1:
        violations.append("SessionStart smoke command must appear exactly once")
    runner_test_matches = [
        (job, step)
        for job in workflow_jobs(text)
        for step in job.steps
        if step.get("run") == EXPECTED_WORKFLOW_RUNNER_TEST_COMMAND
    ]
    if len(runner_test_matches) != 1:
        violations.append("CI must execute the exact SessionStart runner tests once")
    if len(runner_test_matches) == 1:
        test_job, test_step = runner_test_matches[0]
        violations.extend(
            execution_violations("SessionStart runner test job", test_job.fields, {})
        )
        violations.extend(
            execution_violations(
                "SessionStart runner test step",
                test_step,
                test_job.inherited_execution_fields,
            )
        )
    if text.count(EXPECTED_WORKFLOW_RUNNER_TEST_COMMAND) != 1:
        violations.append("SessionStart runner test command must appear exactly once")
    return violations


class RepositoryCiGateTests(unittest.TestCase):
    def setUp(self) -> None:
        self.root = Path(__file__).resolve().parents[2]
        workflow = (self.root / ".github/workflows/ci.yml").read_text(encoding="utf-8")
        self.jobs = {job.job_id: job for job in workflow_jobs(workflow)}

    def test_runtime_checks_do_not_depend_on_benchmark_evidence(self) -> None:
        for job_id in ("runtime_checks", "benchmark_evidence"):
            with self.subTest(job=job_id):
                job = self.jobs[job_id]
                self.assertNotIn("needs", job.fields)
                self.assertEqual(execution_violations(job_id, job.fields, {}), [])

        runtime_commands = [step.get("run") for step in self.jobs["runtime_checks"].steps]
        for command in (
            EXPECTED_WORKFLOW_SMOKE_COMMAND,
            EXPECTED_WORKFLOW_RUNNER_TEST_COMMAND,
            "cargo fmt --check",
            "cargo check --no-default-features --bin remem-hook",
            "cargo clippy --all-targets -- -D warnings",
            "scripts/smoke_native_web_api.sh",
            "cargo test --no-default-features --features local-onnx",
            "cargo test --features eval --lib eval --test e2e_eval",
        ):
            with self.subTest(command=command):
                self.assertEqual(runtime_commands.count(command), 1)

        evidence_commands = [
            step.get("run") for step in self.jobs["benchmark_evidence"].steps
        ]
        for command in (
            'cargo run --locked -- bench verify --root eval/public --json-out "$RUNNER_TEMP/remem-public-bench-verify.json"',
            'python3 scripts/ci/check_public_claims.py --verdict "$RUNNER_TEMP/remem-public-bench-verify.json"',
            "cargo run -- eval-extraction --json --check-baseline > /tmp/remem-extraction-eval.json",
            "cargo run -- eval-gates --json-out /tmp/remem-eval-gates.json",
        ):
            with self.subTest(command=command):
                self.assertEqual(evidence_commands.count(command), 1)
                self.assertNotIn(command, runtime_commands)

    def test_check_always_aggregates_every_required_ci_job(self) -> None:
        check = self.jobs["check"]
        self.assertEqual(check.fields.get("name"), "check")
        self.assertEqual(check.fields.get("if"), "${{ always() }}")
        self.assertEqual(check.fields.get("continue-on-error", "false"), "false")
        needs = check.fields.get("needs", "")
        self.assertTrue(needs.startswith("[") and needs.endswith("]"))
        dependencies = [yaml_scalar(item) for item in needs[1:-1].split(",")]
        self.assertEqual(set(dependencies), REQUIRED_CI_JOBS)
        self.assertEqual(len(dependencies), len(REQUIRED_CI_JOBS))
        for job_id in REQUIRED_CI_JOBS:
            self.assertIn(job_id, self.jobs)
            self.assertEqual(
                self.jobs[job_id].fields.get("continue-on-error", "false"), "false"
            )
        self.assertEqual(len(check.steps), 1)
        self.assertEqual(check.steps[0].get("shell"), "bash")
        self.assertEqual(check.steps[0].get("run"), "|")
        self.assertEqual(
            execution_violations("CI aggregation step", check.steps[0], {}), []
        )

    def test_check_shell_rejects_every_unsuccessful_dependency(self) -> None:
        check = self.jobs["check"]
        scripts = re.findall(
            r"^        run: \|\n((?:          .*\n|\n)+)", check.source, re.MULTILINE
        )
        self.assertEqual(len(scripts), 1)
        script = textwrap.dedent(scripts[0])
        bindings = re.findall(
            r"^          ([A-Z_]+): \$\{\{ needs\.([A-Za-z0-9_-]+)\.result \}\}$",
            check.source,
            re.MULTILINE,
        )
        self.assertEqual(len(bindings), len(REQUIRED_CI_JOBS))
        self.assertEqual(len(dict(bindings)), len(bindings))
        self.assertEqual({job_id for _, job_id in bindings}, REQUIRED_CI_JOBS)
        cases = [(None, "success")] + [
            (job_id, outcome)
            for job_id in sorted(REQUIRED_CI_JOBS)
            for outcome in ("failure", "cancelled", "skipped", "")
        ]
        for unsuccessful_job, outcome in cases:
            with self.subTest(job=unsuccessful_job, outcome=outcome):
                env = {"PATH": os.defpath}
                env.update(
                    {
                        name: outcome if job_id == unsuccessful_job else "success"
                        for name, job_id in bindings
                    }
                )
                result = subprocess.run(
                    ["bash", "--noprofile", "--norc", "-e", "-o", "pipefail", "-c", script],
                    cwd=self.root,
                    env=env,
                    text=True,
                    capture_output=True,
                    check=False,
                    timeout=10,
                )
                if unsuccessful_job is None:
                    self.assertEqual(result.returncode, 0, result.stderr)
                else:
                    self.assertNotEqual(result.returncode, 0, result.stderr)


if __name__ == "__main__":
    unittest.main()
