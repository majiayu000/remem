import tempfile
import unittest
from pathlib import Path

import check_documentation_contracts


class DocumentationContractTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp_dir = tempfile.TemporaryDirectory(prefix="remem-doc-contract-")
        self.root = Path(self.temp_dir.name)
        (self.root / "docs/specs/project-memory-pack").mkdir(parents=True)
        (self.root / "scripts/ci").mkdir(parents=True)
        (self.root / "README.md").write_text(
            """# remem

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
""",
            encoding="utf-8",
        )
        (self.root / "README.zh-CN.md").write_text(
            (self.root / "README.md").read_text(encoding="utf-8"),
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
scripts/ci/smoke_sessionstart_context_gate.sh
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

        self.assertEqual(
            workflow.count("run: scripts/ci/smoke_sessionstart_context_gate.sh"), 1
        )


if __name__ == "__main__":
    unittest.main()
