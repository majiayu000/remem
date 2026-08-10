#!/usr/bin/env python3

from __future__ import annotations

import importlib.util
import json
import sys
import unittest
from pathlib import Path


ROOT = Path(__file__).parent
MODULE_PATH = ROOT / "validate_memory_audit.py"
SPEC = importlib.util.spec_from_file_location("validate_memory_audit", MODULE_PATH)
assert SPEC is not None and SPEC.loader is not None
validator = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = validator
SPEC.loader.exec_module(validator)


def fixture(name: str) -> dict:
    return json.loads((ROOT / "fixtures" / name).read_text(encoding="utf-8"))


class ValidateMemoryAuditTests(unittest.TestCase):
    def test_valid_fixture(self) -> None:
        value = fixture("audit_run_valid.json")
        self.assertIs(validator.validate_artifact(value), value)

    def test_duplicate_id_is_rejected(self) -> None:
        with self.assertRaisesRegex(validator.AuditValidationError, "duplicate IDs"):
            validator.validate_artifact(fixture("audit_run_duplicate_id.json"))

    def test_low_confidence_action_is_rejected(self) -> None:
        with self.assertRaisesRegex(validator.AuditValidationError, "must abstain"):
            validator.validate_artifact(fixture("audit_run_low_confidence_action.json"))

    def test_delete_verdict_is_rejected(self) -> None:
        value = fixture("audit_run_valid.json")
        value["records"][0]["verdict"] = "delete"
        with self.assertRaisesRegex(validator.AuditValidationError, "not allowed"):
            validator.validate_artifact(value)

    def test_unredacted_input_is_rejected(self) -> None:
        value = fixture("audit_run_valid.json")
        value["source"]["content_redacted"] = False
        with self.assertRaisesRegex(validator.AuditValidationError, "must be true"):
            validator.validate_artifact(value)


if __name__ == "__main__":
    unittest.main()
