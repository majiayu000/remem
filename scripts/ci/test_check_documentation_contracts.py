import tempfile
import unittest
from pathlib import Path

import check_documentation_contracts


class DocumentationContractTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp_dir = tempfile.TemporaryDirectory(prefix="remem-doc-contract-")
        self.root = Path(self.temp_dir.name)
        (self.root / "docs/specs/project-memory-pack").mkdir(parents=True)
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
[Smoke](sessionstart-context-smoke.md)
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
tmpdir="$(mktemp -d)"
REMEM_DATA_DIR="$tmpdir" remem encrypt
printf '{"session_id":"gate-smoke","cwd":"%s","transcript_path":"/tmp/remem-gate-smoke.jsonl"}' "$PWD" | REMEM_DATA_DIR="$tmpdir" REMEM_CONTEXT_HOST=codex-cli remem context | wc -c
printf '{"session_id":"gate-smoke","cwd":"%s","transcript_path":"/tmp/remem-gate-smoke.jsonl"}' "$PWD" | REMEM_DATA_DIR="$tmpdir" REMEM_CONTEXT_HOST=codex-cli remem context | wc -c
```
<!-- remem-doc-contract:isolated-sessionstart-smoke:end -->
""",
            encoding="utf-8",
        )
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

    def test_rejects_uninitialized_isolated_store(self) -> None:
        smoke = self.root / "docs/sessionstart-context-smoke.md"
        smoke.write_text(
            smoke.read_text(encoding="utf-8").replace(
                'REMEM_DATA_DIR="$tmpdir" remem encrypt\n',
                "",
            ),
            encoding="utf-8",
        )

        violations = check_documentation_contracts.check(self.root)

        self.assertTrue(any("isolated SessionStart" in item for item in violations))

    def test_rejects_nonisolated_canonical_smoke_guide(self) -> None:
        smoke = self.root / "docs/sessionstart-context-smoke.md"
        smoke.write_text(
            smoke.read_text(encoding="utf-8").replace(
                'REMEM_DATA_DIR="$tmpdir" ',
                'REMEM_DATA_DIR="$HOME/.remem" ',
            ),
            encoding="utf-8",
        )

        violations = check_documentation_contracts.check(self.root)

        self.assertTrue(any("isolated SessionStart" in item for item in violations))

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


if __name__ == "__main__":
    unittest.main()
