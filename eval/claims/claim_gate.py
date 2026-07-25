#!/usr/bin/env python3
"""Claim wording gate for eval/claims/registry.json (issue #931).

Offline gate: validates the claim registry schema, enforces verdict rules
(PASS / FAIL / INSUFFICIENT), checks allowed/forbidden wording, and verifies
supporting report hashes when the report file is present on disk.

Usage:
  python3 eval/claims/claim_gate.py check [registry.json]
  python3 eval/claims/claim_gate.py --self-test
"""

import argparse
import hashlib
import json
import re
import sys
import unittest
from pathlib import Path

CLAIMS_DIR = Path(__file__).resolve().parent
DEFAULT_REGISTRY = CLAIMS_DIR / "registry.json"
SCHEMA_PATH = CLAIMS_DIR / "claims-registry.schema.json"

VERDICTS = ("PASS", "FAIL", "INSUFFICIENT")
DIRECTIONAL_PREFIX = "Directional evidence:"


def validate_schema(instance, schema, path="$", root=None):
    """Minimal offline JSON-schema subset validator. Returns list of errors."""
    if root is None:
        root = schema
    if "$ref" in schema:
        target = root
        for part in schema["$ref"].lstrip("#/").split("/"):
            target = target[part]
        return validate_schema(instance, target, path, root)
    errors = []
    if "const" in schema and instance != schema["const"]:
        errors.append(f"{path}: expected const {schema['const']!r}")
    if "enum" in schema and instance not in schema["enum"]:
        errors.append(f"{path}: {instance!r} not in enum {schema['enum']}")
    expected = schema.get("type")
    if expected:
        type_map = {
            "object": dict,
            "array": list,
            "string": str,
            "integer": int,
            "number": (int, float),
            "boolean": bool,
            "null": type(None),
        }
        allowed = expected if isinstance(expected, list) else [expected]
        ok = False
        for name in allowed:
            py = type_map[name]
            if isinstance(instance, py) and not (
                name in ("integer", "number") and isinstance(instance, bool)
            ):
                ok = True
                break
        if not ok:
            errors.append(f"{path}: expected type {expected}, got {type(instance).__name__}")
            return errors
    if isinstance(instance, dict):
        for key in schema.get("required", []):
            if key not in instance:
                errors.append(f"{path}: missing required key {key!r}")
        props = schema.get("properties", {})
        if schema.get("additionalProperties") is False:
            for key in instance:
                if key not in props:
                    errors.append(f"{path}: unexpected key {key!r}")
        for key, subschema in props.items():
            if key in instance:
                errors.extend(validate_schema(instance[key], subschema, f"{path}.{key}", root))
    if isinstance(instance, list):
        if "minItems" in schema and len(instance) < schema["minItems"]:
            errors.append(f"{path}: fewer than {schema['minItems']} items")
        if "items" in schema:
            for i, item in enumerate(instance):
                errors.extend(validate_schema(item, schema["items"], f"{path}[{i}]", root))
    if isinstance(instance, str):
        if "minLength" in schema and len(instance) < schema["minLength"]:
            errors.append(f"{path}: string shorter than {schema['minLength']}")
        if "pattern" in schema and not re.search(schema["pattern"], instance):
            errors.append(f"{path}: does not match pattern {schema['pattern']!r}")
    if isinstance(instance, (int, float)) and not isinstance(instance, bool):
        if "minimum" in schema and instance < schema["minimum"]:
            errors.append(f"{path}: {instance} < minimum {schema['minimum']}")
        if "exclusiveMinimum" in schema and instance <= schema["exclusiveMinimum"]:
            errors.append(f"{path}: {instance} <= exclusiveMinimum {schema['exclusiveMinimum']}")
    return errors


def sha256_file(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def gate_errors(registry, repo_root):
    """Wording/verdict/report-hash rules beyond plain schema validation."""
    errors = []
    seen_ids = set()
    for i, claim in enumerate(registry.get("claims", [])):
        where = f"claims[{i}] ({claim.get('id', '?')})"
        cid = claim.get("id")
        if cid in seen_ids:
            errors.append(f"{where}: duplicate claim id")
        seen_ids.add(cid)
        status = claim.get("status")
        forbidden = [w.lower() for w in claim.get("forbidden_wording", [])]
        for phrase in claim.get("allowed_wording", []):
            lower = phrase.lower()
            for bad in forbidden:
                if bad in lower:
                    errors.append(
                        f"{where}: allowed wording contains forbidden phrase {bad!r}: {phrase!r}"
                    )
            if status == "INSUFFICIENT" and not phrase.startswith(DIRECTIONAL_PREFIX):
                errors.append(
                    f"{where}: INSUFFICIENT wording must start with "
                    f"{DIRECTIONAL_PREFIX!r}: {phrase!r}"
                )
        report = claim.get("supporting_report")
        if status in ("PASS", "FAIL"):
            if not report:
                errors.append(f"{where}: status {status} requires supporting_report")
            else:
                report_path = Path(repo_root) / report["path"]
                if not report_path.is_file():
                    errors.append(f"{where}: supporting report missing: {report['path']}")
                elif sha256_file(report_path) != report["sha256"]:
                    errors.append(f"{where}: supporting report sha256 mismatch: {report['path']}")
        elif status == "INSUFFICIENT" and report is not None:
            errors.append(f"{where}: INSUFFICIENT must not cite a supporting_report")
    return errors


def check(registry_path, schema_path=SCHEMA_PATH, repo_root=None):
    registry_path = Path(registry_path)
    if repo_root is None:
        repo_root = registry_path.resolve().parent.parent.parent
    registry = json.loads(registry_path.read_text())
    schema = json.loads(Path(schema_path).read_text())
    errors = validate_schema(registry, schema)
    if not errors:
        errors = gate_errors(registry, repo_root)
    return errors


class ClaimGateTests(unittest.TestCase):
    @staticmethod
    def base_claim(**overrides):
        claim = {
            "id": "test-claim",
            "comparison": {"treatment": "remem_e2e", "control": "no_memory"},
            "metric": "resolved_rate",
            "gate": {"min_effect_pp": 10, "ci_lower_bound_pp_gt": 0},
            "status": "INSUFFICIENT",
            "allowed_wording": ["Directional evidence: internal fixture only."],
            "forbidden_wording": ["proves", "state of the art"],
            "supporting_report": None,
        }
        claim.update(overrides)
        return claim

    def registry(self, *claims):
        return {"schema_version": 1, "issue": "#931", "locked": False, "claims": list(claims)}

    def test_valid_insufficient_claim_passes(self):
        reg = self.registry(self.base_claim())
        schema = json.loads(SCHEMA_PATH.read_text())
        self.assertEqual(validate_schema(reg, schema), [])
        self.assertEqual(gate_errors(reg, CLAIMS_DIR), [])

    def test_forbidden_wording_rejected(self):
        claim = self.base_claim(
            allowed_wording=["Directional evidence: this PROVES remem wins."]
        )
        errs = gate_errors(self.registry(claim), CLAIMS_DIR)
        self.assertTrue(any("forbidden phrase" in e for e in errs))

    def test_insufficient_requires_directional_prefix(self):
        claim = self.base_claim(allowed_wording=["remem improves resolved rate"])
        errs = gate_errors(self.registry(claim), CLAIMS_DIR)
        self.assertTrue(any("must start with" in e for e in errs))

    def test_pass_requires_supporting_report(self):
        claim = self.base_claim(status="PASS", allowed_wording=[])
        errs = gate_errors(self.registry(claim), CLAIMS_DIR)
        self.assertTrue(any("requires supporting_report" in e for e in errs))

    def test_pass_report_hash_verified(self):
        report = CLAIMS_DIR / "registry.json"
        good = sha256_file(report)
        claim = self.base_claim(
            status="PASS",
            allowed_wording=[],
            supporting_report={"path": "eval/claims/registry.json", "sha256": good},
        )
        root = CLAIMS_DIR.parent.parent
        self.assertEqual(gate_errors(self.registry(claim), root), [])
        claim["supporting_report"]["sha256"] = "0" * 64
        errs = gate_errors(self.registry(claim), root)
        self.assertTrue(any("sha256 mismatch" in e for e in errs))

    def test_bad_status_rejected_by_schema(self):
        reg = self.registry(self.base_claim(status="MAYBE"))
        schema = json.loads(SCHEMA_PATH.read_text())
        self.assertTrue(any("not in enum" in e for e in validate_schema(reg, schema)))

    def test_checked_in_registry_passes_gate(self):
        self.assertEqual(check(DEFAULT_REGISTRY), [])


def main(argv):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", nargs="?", default="check", choices=["check"])
    parser.add_argument("registry", nargs="?", default=str(DEFAULT_REGISTRY))
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args(argv)
    if args.self_test:
        suite = unittest.defaultTestLoader.loadTestsFromTestCase(ClaimGateTests)
        result = unittest.TextTestRunner(verbosity=2).run(suite)
        return 0 if result.wasSuccessful() else 1
    errors = check(args.registry)
    if errors:
        for error in errors:
            print(f"FAIL {error}")
        return 1
    print(f"OK claim gate passed: {args.registry}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
