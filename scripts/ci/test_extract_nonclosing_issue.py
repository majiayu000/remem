#!/usr/bin/env python3
"""Focused tests for the non-closing issue relation adapter."""

from __future__ import annotations

import importlib.util
import unittest
from pathlib import Path


SCRIPT = Path(__file__).with_name("extract_nonclosing_issue.py")
SPEC = importlib.util.spec_from_file_location("extract_nonclosing_issue", SCRIPT)
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


def snapshot(body: str, closing: list[dict[str, int]] | None = None) -> dict:
    return {"body": body, "closingIssuesReferences": closing or []}


class NonClosingIssueTests(unittest.TestCase):
    def test_one_visible_refs_directive_passes(self) -> None:
        self.assertEqual(MODULE.extract_issue(snapshot("## Issue Links\n\nRefs #813")), 813)

    def test_closing_relation_fails(self) -> None:
        with self.assertRaisesRegex(MODULE.EvidenceError, "non-closing"):
            MODULE.extract_issue(snapshot("Refs #813", [{"number": 813}]))

    def test_missing_or_duplicate_visible_refs_fail(self) -> None:
        for body in ("No issue", "Refs #813\nRefs #814"):
            with self.subTest(body=body):
                with self.assertRaisesRegex(MODULE.EvidenceError, "exactly one"):
                    MODULE.extract_issue(snapshot(body))

    def test_code_and_comment_refs_do_not_count(self) -> None:
        for body in ("```\nRefs #813\n```", "<!-- Refs #813 -->"):
            with self.subTest(body=body):
                with self.assertRaisesRegex(MODULE.EvidenceError, "exactly one"):
                    MODULE.extract_issue(snapshot(body))

    def test_snapshot_shape_and_body_fail_closed(self) -> None:
        for payload in (
            {},
            {"body": "Refs #813", "closingIssuesReferences": [], "number": 908},
            {"body": None, "closingIssuesReferences": []},
        ):
            with self.subTest(payload=payload):
                with self.assertRaises(MODULE.EvidenceError):
                    MODULE.extract_issue(payload)


if __name__ == "__main__":
    unittest.main()
