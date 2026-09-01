import os
import re
import subprocess
import tempfile
import unittest
from dataclasses import dataclass, field
from pathlib import Path

import check_documentation_contracts


EXPECTED_WORKFLOW_SMOKE_COMMAND = (
    "python3 scripts/ci/run_sessionstart_context_gate_smoke.py"
)
EXPECTED_WORKFLOW_RUNNER_TEST_COMMAND = (
    "python3 scripts/ci/test_run_sessionstart_context_gate_smoke.py"
)
SAFE_SHELLS = {"", "bash"}
SAFE_WORKING_DIRECTORIES = {"", ".", "${{ github.workspace }}"}
VALID_BILINGUAL_SURFACE = """
brew install majiayu000/tap/remem
curl -fsSL https://raw.githubusercontent.com/majiayu000/remem/main/install.sh
npm install -g @remem-ai/remem
cargo install remem-ai --bin remem
remem doctor
remem status
remem search "last decision"
docs/README.md
SECURITY.md
docs/specs/SPEC-web-api.md
docs/specs/README.md
CHANGELOG.md
CONTRIBUTING.md
remem install --target cursor
127.0.0.1
Authorization: Bearer
The REST API binds to `127.0.0.1` and requires a bearer token.
remem uninstall --dry-run
REMEM_DATA_DIR
The encrypted database remains in the configured `REMEM_DATA_DIR`.
directional_only_no_public_claim
assets/remem-recall-demo.gif
"""


@dataclass
class WorkflowJob:
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
        if re.fullmatch(r"  [A-Za-z0-9_-]+:\s*", line):
            current_job = WorkflowJob()
            jobs.append(current_job)
            current_step = None
            continue
        if current_job is None:
            continue
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


class DocumentationContractTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp_dir = tempfile.TemporaryDirectory(prefix="remem-doc-contract-")
        self.root = Path(self.temp_dir.name)
        (self.root / "docs/specs/project-memory-pack").mkdir(parents=True)
        (self.root / "scripts/ci").mkdir(parents=True)
        (self.root / "README.md").write_text(
            f"""# remem

<!-- remem-doc-contract:current-project-export:start -->
```bash
remem export --markdown --output ./remem-memory
remem export --pack .remem-pack
```
<!-- remem-doc-contract:current-project-export:end -->

```bash
\"$(brew --prefix remem)/bin/remem\" install --target codex
```

[SessionStart smoke](scripts/ci/smoke_sessionstart_context_gate.sh)

{VALID_BILINGUAL_SURFACE}
""",
            encoding="utf-8",
        )
        (self.root / "README.zh-CN.md").write_text(
            (self.root / "README.md")
            .read_text(encoding="utf-8")
            .replace(
                "The encrypted database remains in the configured `REMEM_DATA_DIR`.",
                "加密数据库会保留在配置的 `REMEM_DATA_DIR`。",
            )
            .replace(
                "The REST API binds to `127.0.0.1` and requires a bearer token.",
                "REST API 只绑定 `127.0.0.1`，并要求 bearer token。",
            ),
            encoding="utf-8",
        )
        (self.root / "docs/installation.md").write_text(
            """# Installation

```bash
\"$(brew --prefix remem)/bin/remem\" install --target codex
```

## Upgrade an existing installation

```bash
/old/path/remem uninstall
brew uninstall remem
npm uninstall -g @remem-ai/remem
cargo uninstall remem-ai
rm /exact/path/to/old/remem
/new/path/remem install --target codex
```
""",
            encoding="utf-8",
        )
        (self.root / "docs/ARCHITECTURE.md").write_text(
            "# Architecture\n\n## Context Injection SessionStart context\n",
            encoding="utf-8",
        )
        (self.root / "docs/README.md").write_text(
            """# Documentation

[Context](ARCHITECTURE.md#context-injection-sessionstart-context)
[Smoke fixture](../scripts/ci/smoke_sessionstart_context_gate.sh)
[Smoke guide](sessionstart-context-smoke.md)
""",
            encoding="utf-8",
        )
        (self.root / "docs/memory-lifecycle.md").write_text(
            """# Memory lifecycle

<!-- remem-doc-contract:memories-fts-lifecycle:start -->
| Invariant | Value |
|---|---|
| Indexed statuses | active, stale, archived |
| Lifecycle visibility | post-JOIN query-time filter |
<!-- remem-doc-contract:memories-fts-lifecycle:end -->
""",
            encoding="utf-8",
        )
        (self.root / "docs/sessionstart-context-smoke.md").write_text(
            """# SessionStart context smoke

<!-- remem-doc-contract:isolated-sessionstart-smoke:start -->
```bash
python3 scripts/ci/run_sessionstart_context_gate_smoke.py
```
<!-- remem-doc-contract:isolated-sessionstart-smoke:end -->

[Fixture](../scripts/ci/smoke_sessionstart_context_gate.sh)
""",
            encoding="utf-8",
        )
        smoke_script = self.root / "scripts/ci/smoke_sessionstart_context_gate.sh"
        smoke_script.write_text(
            "#!/usr/bin/env bash\ntmpdir=fixture\nprintf '%s\\n' \"${tmpdir}\"\n",
            encoding="utf-8",
        )
        smoke_script.chmod(0o755)
        (self.root / "docs/specs/project-memory-pack/PRODUCT.md").write_text(
            """# Project memory pack

<!-- remem-doc-contract:current-project-export:start -->
```bash
remem export --pack .remem-pack/
```
<!-- remem-doc-contract:current-project-export:end -->
""",
            encoding="utf-8",
        )

    def tearDown(self) -> None:
        self.temp_dir.cleanup()

    def local_link_violations(self, addition: str, path: str = "README.md") -> list[str]:
        source = self.root / path
        source.write_text(
            source.read_text(encoding="utf-8") + addition, encoding="utf-8"
        )
        violations: list[str] = []
        check_documentation_contracts.check_local_markdown_links(
            self.root, violations
        )
        return violations

    def bilingual_violations_after_replace(self, path: str, old: str, new: str) -> list[str]:
        readme = self.root / path
        readme.write_text(
            readme.read_text(encoding="utf-8").replace(old, new), encoding="utf-8"
        )
        violations: list[str] = []
        check_documentation_contracts.check_bilingual_readme_invariants(
            self.root, violations
        )
        return violations

    def test_valid_contract_has_no_violations(self) -> None:
        self.assertEqual(check_documentation_contracts.check(self.root), [])

    def test_rejects_path_resolved_homebrew_installer(self) -> None:
        readme = self.root / "README.md"
        readme.write_text(
            readme.read_text(encoding="utf-8").replace(
                '"$(brew --prefix remem)/bin/remem" install --target codex',
                'REMEM_INSTALL_BINARY="$(brew --prefix remem)/bin/remem" remem install --target codex',
            ),
            encoding="utf-8",
        )

        violations = check_documentation_contracts.check(self.root)

        self.assertTrue(any("Homebrew" in item for item in violations))

    def test_rejects_explicit_current_directory_export(self) -> None:
        readme = self.root / "README.md"
        readme.write_text(
            readme.read_text(encoding="utf-8").replace(
                "remem export --pack .remem-pack",
                'remem export --project "$PWD" --pack .remem-pack',
            ),
            encoding="utf-8",
        )

        violations = check_documentation_contracts.check(self.root)

        self.assertTrue(any("canonicalize" in item for item in violations))

    def test_rejects_any_explicit_project_argument_in_current_project_export(self) -> None:
        product = self.root / "docs/specs/project-memory-pack/PRODUCT.md"
        product.write_text(
            product.read_text(encoding="utf-8").replace(
                "remem export --pack .remem-pack/",
                'remem export --project "$(pwd)" --pack .remem-pack/',
            ),
            encoding="utf-8",
        )

        violations = check_documentation_contracts.check(self.root)

        self.assertTrue(any("canonicalize" in item for item in violations))

    def test_rejects_missing_local_anchor(self) -> None:
        hub = self.root / "docs/README.md"
        hub.write_text(
            hub.read_text(encoding="utf-8").replace(
                "#context-injection-sessionstart-context",
                "#context-injection--sessionstart-context",
            ),
            encoding="utf-8",
        )

        violations = check_documentation_contracts.check(self.root)

        self.assertTrue(any("missing Markdown anchor" in item for item in violations))

    def test_local_links_accept_spaces_unicode_duplicates_and_ignored_targets(self) -> None:
        guide = self.root / "docs/Guide With Space.md"
        guide.write_text(
            """# Guide

## Пример Θ 中文

## Repeat

## Repeat
""",
            encoding="utf-8",
        )
        readme = self.root / "README.md"
        readme.write_text(
            readme.read_text(encoding="utf-8")
            + """
[Unicode](<docs/Guide With Space.md#пример-θ-中文>)
[Duplicate](docs/Guide%20With%20Space.md#repeat-1 "title")
[External](https://example.com/missing.md)
[Mail](mailto:docs@example.com)
[Custom](app://documentation)

```markdown
[Fenced false positive](docs/missing.md#not-real)
```
""",
            encoding="utf-8",
        )

        violations: list[str] = []
        check_documentation_contracts.check_local_markdown_links(
            self.root, violations
        )

        self.assertEqual(violations, [])

    def test_local_links_report_source_line_target_and_missing_file(self) -> None:
        violations = self.local_link_violations("\n[Broken](docs/missing.md)\n")
        self.assertTrue(
            any(
                item.startswith("README.md:")
                and ": docs/missing.md:" in item
                and "missing local Markdown target" in item
                for item in violations
            )
        )

    def test_local_links_validate_outer_destination_of_wrapped_image(self) -> None:
        violations = self.local_link_violations(
            "\n[![Build](https://example.com/badge.svg)](docs/missing-wrapper.md)\n"
        )
        self.assertTrue(
            any(
                item.startswith("README.md:")
                and ": docs/missing-wrapper.md:" in item
                and "missing local Markdown target" in item
                for item in violations
            )
        )

    def test_local_links_keep_same_length_fence_with_info_string_open(self) -> None:
        violations = self.local_link_violations(
            """
```markdown
```python
[Still fenced](docs/missing-inside-fence.md)
```
"""
        )
        self.assertFalse(
            any("docs/missing-inside-fence.md" in item for item in violations)
        )

    def test_local_links_do_not_treat_four_space_indent_as_fence(self) -> None:
        violations = self.local_link_violations(
            """
    ```markdown
[Live link](docs/missing-after-indented-code.md)
"""
        )
        self.assertTrue(any("missing-after-indented-code.md" in item for item in violations))

    def test_local_links_support_balanced_parentheses_in_destinations(self) -> None:
        (self.root / "docs/guide(v2).md").write_text("# Guide\n", encoding="utf-8")
        violations = self.local_link_violations("\n[Guide](docs/guide(v2).md)\n")
        self.assertFalse(any("guide(v2" in item for item in violations))

    def test_local_links_support_setext_heading_anchors(self) -> None:
        (self.root / "docs/setext.md").write_text(
            "Installation\n------------\n", encoding="utf-8"
        )
        violations = self.local_link_violations(
            "\n[Install](docs/setext.md#installation)\n"
        )
        self.assertFalse(any("docs/setext.md#installation" in item for item in violations))

    def test_local_links_validate_wrapped_reference_destinations(self) -> None:
        violations = self.local_link_violations(
            """
[guide]:
  docs/missing-wrapped-reference.md

[Guide][guide]
"""
        )
        self.assertTrue(any("missing-wrapped-reference.md" in item for item in violations))

    def test_local_link_fragments_are_case_sensitive(self) -> None:
        violations = self.local_link_violations("\n[Wrong case](#Remem)\n")
        self.assertTrue(
            any(
                "#Remem: missing Markdown anchor Remem in README.md" in item
                for item in violations
            )
        )

    def test_local_links_check_chinese_readme_fragments(self) -> None:
        violations = self.local_link_violations(
            "\n[坏锚点](#不存在)\n", "README.zh-CN.md"
        )
        self.assertTrue(
            any(
                item.startswith("README.zh-CN.md:")
                and ": #不存在:" in item
                and "missing Markdown anchor" in item
                for item in violations
            )
        )

    def test_github_slug_preserves_hyphen_runs_and_removes_other_whitespace(self) -> None:
        self.assertEqual(
            check_documentation_contracts.github_slug("Foo --- Bar\tΘ 中文"),
            "foo-----barθ-中文",
        )

    def test_github_slug_preserves_underscores(self) -> None:
        slug = check_documentation_contracts.github_slug("`captured_events`")
        self.assertEqual(slug, "captured_events")

    def test_github_slug_uses_rendered_inline_heading_text(self) -> None:
        self.assertEqual(check_documentation_contracts.github_slug("_remem_"), "remem")
        self.assertEqual(check_documentation_contracts.github_slug("&lt;name&gt;"), "name")

    def test_bilingual_invariants_report_missing_english_fact(self) -> None:
        violations = self.bilingual_violations_after_replace(
            "README.md", "directional_only_no_public_claim", "directional_only"
        )
        self.assertTrue(
            any(
                item.startswith("README.md:")
                and "public benchmark claim boundary" in item
                for item in violations
            )
        )

    def test_bilingual_invariants_report_missing_chinese_fact(self) -> None:
        violations = self.bilingual_violations_after_replace(
            "README.zh-CN.md",
            "docs/specs/SPEC-web-api.md",
            "docs/specs/old-api.md",
        )
        self.assertTrue(
            any(
                item.startswith("README.zh-CN.md:")
                and "current API contract" in item
                for item in violations
            )
        )

    def test_bilingual_invariants_require_affirmative_data_retention(self) -> None:
        violations = self.bilingual_violations_after_replace(
            "README.md",
            "The encrypted database remains in the configured `REMEM_DATA_DIR`.",
            "The encrypted database is deleted from `REMEM_DATA_DIR`.",
        )
        self.assertTrue(
            any(
                item.startswith("README.md:")
                and "safe uninstall and data retention" in item
                for item in violations
            )
        )

    def test_bilingual_invariants_require_affirmative_api_authentication(self) -> None:
        violations = self.bilingual_violations_after_replace(
            "README.md",
            "The REST API binds to `127.0.0.1` and requires a bearer token.",
            "The REST API binds to `127.0.0.1` and does not require a bearer token.",
        )
        self.assertTrue(
            any(
                item.startswith("README.md:")
                and "localhost bearer-token API" in item
                for item in violations
            )
        )

    def test_rejects_missing_executable_smoke_fixture(self) -> None:
        (self.root / "scripts/ci/smoke_sessionstart_context_gate.sh").unlink()

        violations = check_documentation_contracts.check(self.root)

        self.assertTrue(any("must exist and be executable" in item for item in violations))

    def test_rejects_smoke_guide_that_does_not_route_to_fixture(self) -> None:
        smoke = self.root / "docs/sessionstart-context-smoke.md"
        smoke.write_text(
            smoke.read_text(encoding="utf-8").replace(
                "scripts/ci/smoke_sessionstart_context_gate.sh",
                "scripts/ci/another-smoke.sh",
            ),
            encoding="utf-8",
        )

        violations = check_documentation_contracts.check(self.root)

        self.assertTrue(any("route SessionStart smoke" in item for item in violations))

    def test_rejects_readme_without_smoke_fixture_route(self) -> None:
        readme = self.root / "README.md"
        readme.write_text(
            readme.read_text(encoding="utf-8").replace(
                "scripts/ci/smoke_sessionstart_context_gate.sh",
                "docs/sessionstart-context-smoke.md",
            ),
            encoding="utf-8",
        )

        violations = check_documentation_contracts.check(self.root)

        self.assertTrue(any(item.startswith("README.md: route") for item in violations))

    def test_rejects_context_argument_drift_hidden_in_smoke_guide(self) -> None:
        smoke = self.root / "docs/sessionstart-context-smoke.md"
        smoke.write_text(
            smoke.read_text(encoding="utf-8")
            + "\n```bash\nprintf '{}' | remem context --force | wc -c\n```\n",
            encoding="utf-8",
        )

        violations = check_documentation_contracts.check(self.root)

        self.assertTrue(any("SessionStart" in item for item in violations))

    def test_equivalent_shell_variable_spelling_is_not_a_document_contract(self) -> None:
        fixture = self.root / "scripts/ci/smoke_sessionstart_context_gate.sh"
        self.assertIn("${tmpdir}", fixture.read_text(encoding="utf-8"))

        violations = check_documentation_contracts.check(self.root)

        self.assertEqual(violations, [])

    def test_rejects_active_only_fts_description(self) -> None:
        lifecycle = self.root / "docs/memory-lifecycle.md"
        lifecycle.write_text(
            "# Memory lifecycle\n\nOnly active rows enter the FTS index.\n",
            encoding="utf-8",
        )

        violations = check_documentation_contracts.check(self.root)

        self.assertTrue(any("all-status FTS" in item for item in violations))

    def test_rejects_negated_all_status_fts_description(self) -> None:
        lifecycle = self.root / "docs/memory-lifecycle.md"
        lifecycle.write_text(
            lifecycle.read_text(encoding="utf-8").replace(
                "active, stale, archived",
                "does not index active, stale, archived",
            ),
            encoding="utf-8",
        )

        violations = check_documentation_contracts.check(self.root)

        self.assertTrue(any("all-status FTS" in item for item in violations))


class RepositoryDocumentationContractTests(unittest.TestCase):
    def test_repository_documentation_contract(self) -> None:
        root = Path(__file__).resolve().parents[2]
        self.assertEqual(check_documentation_contracts.check(root), [])

    def test_ci_executes_the_canonical_sessionstart_smoke_fixture(self) -> None:
        root = Path(__file__).resolve().parents[2]
        workflow = (root / ".github/workflows/ci.yml").read_text(encoding="utf-8")

        self.assertEqual(workflow_smoke_registration_violations(workflow), [])

    def test_ci_registration_rejects_disabled_sessionstart_smoke_step(self) -> None:
        root = Path(__file__).resolve().parents[2]
        workflow = (root / ".github/workflows/ci.yml").read_text(encoding="utf-8")
        mutated = workflow.replace(
            f"        run: {EXPECTED_WORKFLOW_SMOKE_COMMAND}",
            "        if: ${{ false }}\n"
            f"        run: {EXPECTED_WORKFLOW_SMOKE_COMMAND}",
        )

        self.assertIn(
            "SessionStart smoke step must be unconditional",
            workflow_smoke_registration_violations(mutated),
        )

    def test_ci_registration_rejects_noop_in_place_of_smoke_fixture(self) -> None:
        root = Path(__file__).resolve().parents[2]
        workflow = (root / ".github/workflows/ci.yml").read_text(encoding="utf-8")
        mutated = workflow.replace(EXPECTED_WORKFLOW_SMOKE_COMMAND, "true")

        self.assertIn(
            "CI must execute the exact SessionStart smoke command once",
            workflow_smoke_registration_violations(mutated),
        )

    def test_ci_registration_rejects_missing_runner_tests(self) -> None:
        root = Path(__file__).resolve().parents[2]
        workflow = (root / ".github/workflows/ci.yml").read_text(encoding="utf-8")
        mutated = workflow.replace(EXPECTED_WORKFLOW_RUNNER_TEST_COMMAND, "true")

        self.assertIn(
            "CI must execute the exact SessionStart runner tests once",
            workflow_smoke_registration_violations(mutated),
        )

    def test_ci_registration_rejects_job_level_disable(self) -> None:
        root = Path(__file__).resolve().parents[2]
        workflow = (root / ".github/workflows/ci.yml").read_text(encoding="utf-8")
        mutated = workflow.replace("  check:\n", "  check:\n    if: ${{ false }}\n")

        self.assertIn(
            "SessionStart smoke job must be unconditional",
            workflow_smoke_registration_violations(mutated),
        )

    def test_ci_registration_rejects_trailing_job_level_disable(self) -> None:
        root = Path(__file__).resolve().parents[2]
        workflow = (root / ".github/workflows/ci.yml").read_text(encoding="utf-8")
        mutated = workflow.replace(
            "\n  windows_local_embedding_security:",
            "\n    if: ${{ false }}\n  windows_local_embedding_security:",
        )

        self.assertIn(
            "SessionStart smoke job must be unconditional",
            workflow_smoke_registration_violations(mutated),
        )

    def test_ci_registration_rejects_continue_on_error(self) -> None:
        root = Path(__file__).resolve().parents[2]
        workflow = (root / ".github/workflows/ci.yml").read_text(encoding="utf-8")
        mutated = workflow.replace(
            f"        run: {EXPECTED_WORKFLOW_SMOKE_COMMAND}",
            "        continue-on-error: true\n"
            f"        run: {EXPECTED_WORKFLOW_SMOKE_COMMAND}",
        )

        self.assertIn(
            "SessionStart smoke step must fail CI on error",
            workflow_smoke_registration_violations(mutated),
        )

    def test_ci_registration_rejects_noop_shell(self) -> None:
        root = Path(__file__).resolve().parents[2]
        workflow = (root / ".github/workflows/ci.yml").read_text(encoding="utf-8")
        mutated = workflow.replace(
            f"        run: {EXPECTED_WORKFLOW_SMOKE_COMMAND}",
            "        shell: true {0}\n"
            f"        run: {EXPECTED_WORKFLOW_SMOKE_COMMAND}",
        )

        self.assertIn(
            "SessionStart smoke step must use the default or standard bash shell",
            workflow_smoke_registration_violations(mutated),
        )

    def test_ci_registration_rejects_wrong_working_directory_and_timeout(self) -> None:
        root = Path(__file__).resolve().parents[2]
        workflow = (root / ".github/workflows/ci.yml").read_text(encoding="utf-8")
        mutated = workflow.replace(
            f"        run: {EXPECTED_WORKFLOW_SMOKE_COMMAND}",
            "        working-directory: /tmp\n"
            "        timeout-minutes: 1\n"
            f"        run: {EXPECTED_WORKFLOW_SMOKE_COMMAND}",
        )
        violations = workflow_smoke_registration_violations(mutated)

        self.assertIn("SessionStart smoke step must run from the repository root", violations)
        self.assertIn(
            "SessionStart smoke step must not be disabled by a local timeout", violations
        )

    def test_smoke_fixture_rejects_invalid_binary_arguments_before_toolchain_use(self) -> None:
        root = Path(__file__).resolve().parents[2]
        fixture = root / "scripts/ci/smoke_sessionstart_context_gate.sh"
        with tempfile.TemporaryDirectory(prefix="remem-smoke-argv-") as raw_tmp:
            temp_root = Path(raw_tmp)
            non_executable = temp_root / "not-executable"
            non_executable.write_text("not a binary", encoding="utf-8")
            missing = temp_root / "missing-remem"
            env = os.environ.copy()
            env["PATH"] = "/usr/bin:/bin"
            probes = (
                ((), "requires exactly one absolute remem binary path"),
                (("target/debug/remem",), "must be absolute"),
                ((str(missing),), "does not exist"),
                ((str(non_executable),), "is not executable"),
            )
            for arguments, expected in probes:
                with self.subTest(arguments=arguments):
                    result = subprocess.run(
                        [str(fixture), *arguments],
                        cwd=root,
                        env=env,
                        text=True,
                        capture_output=True,
                        check=False,
                    )
                    self.assertNotEqual(result.returncode, 0)
                    self.assertIn(expected, result.stderr)


if __name__ == "__main__":
    unittest.main()
