import tempfile
import unittest
from pathlib import Path

import check_documentation_contracts


def handoff_index(row: str) -> str:
    return (
        "### Current/Historical GH Packet Handoffs\n\n"
        "| Contract | Canonical current packet | Historical packet |\n"
        "|---|---|---|\n"
        f"{row}\n"
    )


class CurrentSpecHandoffContractTests(unittest.TestCase):
    def fixture_root(
        self, *, include_current: bool = True, include_historical: bool = True
    ) -> Path:
        temp_dir = tempfile.TemporaryDirectory()
        self.addCleanup(temp_dir.cleanup)
        root = Path(temp_dir.name)
        (root / "docs/specs").mkdir(parents=True)
        if include_current:
            (root / "docs/specs/GH931").mkdir()
        if include_historical:
            (root / "specs/GH931").mkdir(parents=True)
        return root

    def test_accepts_explicit_historical_packet(self) -> None:
        root = self.fixture_root()
        (root / "docs/specs/README.md").write_text(
            handoff_index(
                "| `GH931` | `docs/specs/GH931/` | `specs/GH931/` |"
            ),
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
            handoff_index(""),
            encoding="utf-8",
        )

        violations: list[str] = []
        check_documentation_contracts.check_current_spec_handoffs(
            root, violations
        )

        self.assertTrue(
            any(
                "overlapping GH931 packets require a structured handoff row" in item
                for item in violations
            )
        )

    def test_rejects_mismatched_current_path(self) -> None:
        root = self.fixture_root()
        (root / "docs/specs/README.md").write_text(
            handoff_index(
                "| `GH931` | `docs/specs/GH932/` | `specs/GH931/` |"
            ),
            encoding="utf-8",
        )

        violations: list[str] = []
        check_documentation_contracts.check_current_spec_handoffs(
            root, violations
        )

        self.assertTrue(any("must declare exact" in item for item in violations))

    def test_rejects_extra_handoff_column(self) -> None:
        root = self.fixture_root()
        (root / "docs/specs/README.md").write_text(
            handoff_index(
                "| `GH931` | `docs/specs/GH931/` | `specs/GH931/` | canonical |"
            ),
            encoding="utf-8",
        )

        violations: list[str] = []
        check_documentation_contracts.check_current_spec_handoffs(
            root, violations
        )

        self.assertTrue(any("must declare exact" in item for item in violations))

    def test_rejects_declared_missing_historical_packet(self) -> None:
        root = self.fixture_root(include_historical=False)
        (root / "docs/specs/README.md").write_text(
            handoff_index(
                "| `GH931` | `docs/specs/GH931/` | `specs/GH931/` |"
            ),
            encoding="utf-8",
        )

        violations: list[str] = []
        check_documentation_contracts.check_current_spec_handoffs(
            root, violations
        )

        self.assertIn(
            "docs/specs/README.md: declared historical packet `specs/GH931/` is missing",
            violations,
        )

    def test_rejects_declared_missing_current_packet(self) -> None:
        root = self.fixture_root(include_current=False)
        (root / "docs/specs/README.md").write_text(
            handoff_index(
                "| `GH931` | `docs/specs/GH931/` | `specs/GH931/` |"
            ),
            encoding="utf-8",
        )

        violations: list[str] = []
        check_documentation_contracts.check_current_spec_handoffs(
            root, violations
        )

        self.assertIn(
            "docs/specs/README.md: declared current packet `docs/specs/GH931/` is missing",
            violations,
        )
