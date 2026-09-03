import tempfile
import unittest
from pathlib import Path

import check_documentation_contracts


class CurrentSpecHandoffContractTests(unittest.TestCase):
    def fixture_root(self) -> Path:
        temp_dir = tempfile.TemporaryDirectory()
        self.addCleanup(temp_dir.cleanup)
        root = Path(temp_dir.name)
        (root / "docs/specs/GH931").mkdir(parents=True)
        (root / "specs/GH931").mkdir(parents=True)
        return root

    def test_accepts_explicit_historical_packet(self) -> None:
        root = self.fixture_root()
        (root / "docs/specs/README.md").write_text(
            "| `GH931/` | Current contract | Canonical contract; historical "
            "planning packet: `specs/GH931/`. |\n",
            encoding="utf-8",
        )

        violations: list[str] = []
        check_documentation_contracts.check_current_spec_handoffs(
            root, violations
        )

        self.assertEqual(violations, [])

    def test_rejects_unlabelled_duplicate_tree(self) -> None:
        root = self.fixture_root()
        (root / "docs/specs/README.md").write_text(
            "| `GH931/` | Current contract | Refs #931. |\n",
            encoding="utf-8",
        )

        violations: list[str] = []
        check_documentation_contracts.check_current_spec_handoffs(
            root, violations
        )

        self.assertTrue(
            any(
                "current GH931/ must link `specs/GH931/` and label it historical"
                in item
                for item in violations
            )
        )
