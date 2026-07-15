#!/usr/bin/env python3
"""Regression tests for the closed published and runtime schema contracts."""

from __future__ import annotations

import hashlib
import json
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path
from unittest import mock


ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "checks"))
from schema_contract import (  # noqa: E402
    ADOPTED_SCHEMA_KEYWORDS,
    KEYWORD_APPLICABLE_TYPES,
    TYPE_SPECIFIC_SCHEMA_KEYWORDS,
    uses_runtime_profile,
    validate_schema_node,
)

PACK_DIRS = ("checks", "policies", "review", "schemas", "skills", "templates", "tools")
PACK_FILES = (
    "AGENT_USAGE.md", "AGENTS.md", "labels.yaml", "skills-lock.json",
    "states.yaml", "workflow.yaml",
)
RUNTIME_SCHEMA_SCRIPT = """
import json
import sys
sys.path.insert(0, sys.argv[1])
from specrail_lib import validate_instance
with open(sys.argv[2], encoding="utf-8") as fh:
    schema = json.load(fh)
validate_instance(schema, json.loads(sys.argv[3]))
"""
RUNTIME_DATA = {
    "issue": 1,
    "collected_at": "now",
    "open_prs_complete": True,
    "open_pr_limit": 10,
    "open_prs": [{"number": 2, "head_ref": "review-fix", "references_issue": True}],
    "remote_branches": [],
}


def run(command: list[str], *, cwd: Path) -> subprocess.CompletedProcess[str]:
    return subprocess.run(command, cwd=cwd, capture_output=True, text=True, check=False)


def assert_passed(completed: subprocess.CompletedProcess[str], label: str) -> None:
    assert completed.returncode == 0, f"{label} failed:\n{completed.stdout}{completed.stderr}"


def copy_pack(repo: Path) -> None:
    for relative_path in PACK_DIRS:
        shutil.copytree(
            ROOT / relative_path, repo / relative_path,
            ignore=shutil.ignore_patterns("__pycache__", "*.pyc"),
        )
    for relative_path in PACK_FILES:
        shutil.copy2(ROOT / relative_path, repo / relative_path)
    target = repo / "scripts" / "sync-specrail-checks.sh"
    target.parent.mkdir(parents=True)
    shutil.copy2(ROOT / "scripts" / "sync-specrail-checks.sh", target)


def write_lock(path: Path, lock: dict[str, object]) -> None:
    path.write_text(json.dumps(lock, indent=2) + "\n", encoding="utf-8")


def update_lock_hash(lock: dict[str, object], relative_path: str, path: Path) -> None:
    entries = lock["files"]
    assert isinstance(entries, list)
    for entry in entries:
        if isinstance(entry, dict) and entry.get("path") == relative_path:
            entry["sha256"] = hashlib.sha256(path.read_bytes()).hexdigest()
            return
    raise AssertionError(f"missing lock entry for {relative_path}")


def set_nested(value: dict[str, object], path: tuple[str, ...], replacement: object) -> None:
    target = value
    for key in path[:-1]:
        child = target[key]
        assert isinstance(child, dict), f"fixture path {path!r} is not an object"
        target = child
    target[path[-1]] = replacement


def run_workflow(repo: Path) -> subprocess.CompletedProcess[str]:
    return run(
        [sys.executable, str(repo / "checks" / "check_workflow.py"), "--repo", str(repo)],
        cwd=repo,
    )


def run_runtime(
    repo: Path, schema_path: Path, data: dict[str, object] = RUNTIME_DATA
) -> subprocess.CompletedProcess[str]:
    return run(
        [sys.executable, "-c", RUNTIME_SCHEMA_SCRIPT, str(repo / "checks"),
         str(schema_path), json.dumps(data)],
        cwd=repo,
    )


def assert_contract_failure(repo: Path, expected: str, label: str) -> None:
    workflow = run_workflow(repo)
    assert workflow.returncode != 0, f"{label} must fail workflow validation"
    assert expected in workflow.stdout, workflow.stdout
    sync = run([str(repo / "scripts" / "sync-specrail-checks.sh"), "--verify"], cwd=repo)
    assert sync.returncode != 0, f"{label} must fail sync validation"
    assert "files match lock" in sync.stdout
    assert expected in sync.stdout, sync.stdout


def collect_keywords(schema: dict[str, object]) -> set[str]:
    found = set(schema)
    for keyword in ("$defs", "dependentSchemas", "patternProperties", "properties"):
        children = schema.get(keyword)
        if isinstance(children, dict):
            for child in children.values():
                if isinstance(child, dict):
                    found.update(collect_keywords(child))
    for keyword in ("allOf", "anyOf", "oneOf", "prefixItems"):
        children = schema.get(keyword)
        if isinstance(children, list):
            for child in children:
                if isinstance(child, dict):
                    found.update(collect_keywords(child))
    for keyword in (
        "additionalProperties", "contains", "contentSchema", "else", "if", "items",
        "not", "propertyNames", "then", "unevaluatedItems", "unevaluatedProperties",
    ):
        child = schema.get(keyword)
        if isinstance(child, dict):
            found.update(collect_keywords(child))
    return found


def assert_vocabulary_and_baselines() -> None:
    values = {keyword: "value" for keyword in (
        "$anchor", "$comment", "$dynamicAnchor", "$dynamicRef", "$id", "$ref", "$schema",
        "contentEncoding", "contentMediaType", "description", "format", "title",
    )}
    values.update({keyword: True for keyword in ("deprecated", "readOnly", "uniqueItems", "writeOnly")})
    values.update({keyword: 0 for keyword in (
        "maxContains", "maxItems", "maxLength", "maxProperties",
        "minContains", "minItems", "minLength", "minProperties",
    )})
    values.update({keyword: 1 for keyword in (
        "exclusiveMaximum", "exclusiveMinimum", "maximum", "minimum", "multipleOf",
    )})
    values.update({keyword: {} for keyword in (
        "$defs", "dependentSchemas", "patternProperties", "properties",
        "contains", "contentSchema", "else", "if", "items", "not", "propertyNames", "then",
    )})
    values.update({keyword: [{}] for keyword in ("allOf", "anyOf", "oneOf", "prefixItems")})
    values.update({keyword: True for keyword in (
        "additionalProperties", "unevaluatedItems", "unevaluatedProperties",
    )})
    values.update({
        "$vocabulary": {"urn:example:vocabulary": True}, "const": None, "default": None,
        "dependentRequired": {}, "enum": [None], "examples": [], "pattern": "^ok$",
        "required": [], "type": "object",
    })
    assert set(values) == ADOPTED_SCHEMA_KEYWORDS
    for keyword, value in values.items():
        assert validate_schema_node({keyword: value}, Path("vocabulary.schema.json")) == []

    expected_typed = frozenset({
        "additionalProperties", "contains", "contentEncoding", "contentMediaType",
        "contentSchema", "dependentRequired", "dependentSchemas", "exclusiveMaximum",
        "exclusiveMinimum", "format", "items", "maxContains", "maxItems", "maxLength",
        "maxProperties", "maximum", "minContains", "minItems", "minLength", "minProperties",
        "minimum", "multipleOf", "pattern", "patternProperties", "prefixItems", "properties",
        "propertyNames", "required", "unevaluatedItems", "unevaluatedProperties", "uniqueItems",
    })
    assert TYPE_SPECIFIC_SCHEMA_KEYWORDS == expected_typed
    assert frozenset(KEYWORD_APPLICABLE_TYPES) == expected_typed

    observed: set[str] = set()
    for path in sorted((ROOT / "schemas").glob("*.schema.json")):
        schema = json.loads(path.read_text(encoding="utf-8"))
        observed.update(collect_keywords(schema))
        assert validate_schema_node(
            schema, path.relative_to(ROOT), runtime_profile=uses_runtime_profile(path)
        ) == []
    assert observed <= ADOPTED_SCHEMA_KEYWORDS


def assert_regex_contracts() -> None:
    path = Path("regex.schema.json")
    assert validate_schema_node({"pattern": "(?<name>a)"}, path) == []
    errors = validate_schema_node({"pattern": "(?P<name>a)"}, path)
    assert any("invalid ECMAScript regex" in error for error in errors)
    assert validate_schema_node({"patternProperties": {"(?<name>a)": True}}, path) == []
    errors = validate_schema_node({"patternProperties": {"(?P<name>a)": True}}, path)
    assert any("invalid ECMAScript regex" in error for error in errors)

    python_only = validate_schema_node(
        {"type": "string", "pattern": "(?P<name>.+)"}, path, runtime_profile=True
    )
    assert any("invalid ECMAScript regex" in error for error in python_only)
    assert not any("invalid Python runtime regex" in error for error in python_only)
    ecmascript_only = validate_schema_node(
        {"type": "string", "pattern": "(?<name>.+)"}, path, runtime_profile=True
    )
    assert not any("invalid ECMAScript regex" in error for error in ecmascript_only)
    assert any("invalid Python runtime regex" in error for error in ecmascript_only)
    with mock.patch.dict(os.environ, {"PATH": ""}):
        missing = validate_schema_node({"pattern": "ok"}, path)
    assert any("ECMAScript regex engine failed" in error for error in missing)


def assert_boolean_children_and_enum_uniqueness() -> None:
    boolean_children = {
        "$defs": {"a": True}, "dependentSchemas": {"a": False},
        "patternProperties": {"^a$": True}, "properties": {"a": False},
        "allOf": [True], "anyOf": [False], "oneOf": [True], "prefixItems": [False],
        "contains": True, "contentSchema": False, "else": True, "if": False,
        "items": True, "not": False, "propertyNames": True, "then": False,
        "additionalProperties": True, "unevaluatedItems": False,
        "unevaluatedProperties": True,
    }
    assert validate_schema_node(boolean_children, Path("boolean.schema.json")) == []
    runtime_errors = validate_schema_node(
        boolean_children, Path("runtime.schema.json"), runtime_profile=True
    )
    assert sum("boolean schema is not executable" in error for error in runtime_errors) == 18
    duplicate = validate_schema_node(
        {"enum": [{"a": [1, 2]}, {"a": [1, 2]}]}, Path("enum.schema.json")
    )
    assert any("enum must contain unique JSON values" in error for error in duplicate)
    assert validate_schema_node({"enum": [True, 1]}, Path("enum.schema.json")) == []
    numeric_duplicate = validate_schema_node({"enum": [1, 1.0]}, Path("enum.schema.json"))
    assert any("enum must contain unique JSON values" in error for error in numeric_duplicate)


def assert_published_contracts() -> None:
    base = {
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "title": "Published contract fixture", "type": "object",
        "properties": {
            "missing_type": {"minLength": 1},
            "mixed_union": {"type": ["string", "null"], "minLength": 1, "pattern": "(?<ok>.+)"},
            "conditional": {
                "if": {"properties": {"kind": {"const": "primary"}}},
                "then": {"required": ["value"]},
                "else": {"properties": {"fallback": {"type": "string"}}},
            },
        },
        "additionalProperties": True,
    }
    negative_cases = (
        ("minLength", {"type": "integer", "minLength": 1}, "minLength is incompatible"),
        ("pattern", {"type": "integer", "pattern": "^x$"}, "pattern is incompatible"),
        ("unevaluatedProperties", {"type": "integer", "unevaluatedProperties": True}, "unevaluatedProperties is incompatible"),
        ("unevaluatedItems", {"type": "string", "unevaluatedItems": True}, "unevaluatedItems is incompatible"),
        ("contentEncoding", {"type": "integer", "contentEncoding": "base64"}, "contentEncoding is incompatible"),
        ("python regex", {"type": "string", "pattern": "(?P<name>a)"}, "invalid ECMAScript regex"),
        ("patternProperties", {"type": "object", "patternProperties": {"[": True}}, "invalid ECMAScript regex"),
    )
    with tempfile.TemporaryDirectory(prefix="remem-published-schema-") as raw:
        repo = Path(raw)
        copy_pack(repo)
        path = repo / "schemas" / "published_fixture.schema.json"
        path.write_text(json.dumps(base, indent=2) + "\n", encoding="utf-8")
        assert_passed(run_workflow(repo), "published positive workflow")
        assert_passed(
            run([str(repo / "scripts" / "sync-specrail-checks.sh"), "--verify"], cwd=repo),
            "published positive sync",
        )
        for label, child, expected in negative_cases:
            malformed = json.loads(json.dumps(base))
            malformed["properties"] = {"value": child}
            path.write_text(json.dumps(malformed, indent=2) + "\n", encoding="utf-8")
            assert_contract_failure(repo, expected, label)


def assert_locked_schema_contracts() -> None:
    cases = (
        ("schemas/duplicate_work_evidence.schema.json", ("minimun",), 1, "$: unsupported JSON Schema keyword 'minimun'"),
        ("schemas/pr_review_gate.schema.json", ("properties", "pr", "minimun"), 1, "$.properties.pr: unsupported JSON Schema keyword 'minimun'"),
        ("schemas/review_result.schema.json", ("properties", "prior_findings", "items", "minimun"), 1, "$.properties.prior_findings.items: unsupported JSON Schema keyword 'minimun'"),
        ("schemas/runtime_checkpoint.schema.json", ("additionalProperties",), {"type": "string", "minimun": 1}, "$.additionalProperties: unsupported JSON Schema keyword 'minimun'"),
        ("schemas/review_result.schema.json", ("properties", "body", "pattern"), "[", "$.properties.body.pattern invalid ECMAScript regex"),
    )
    locked = {case[0] for case in cases}
    with tempfile.TemporaryDirectory(prefix="remem-locked-schema-") as raw:
        repo = Path(raw)
        copy_pack(repo)
        lock_path = repo / "checks" / "specrail-sync.lock.json"
        baseline_lock = json.loads(lock_path.read_text(encoding="utf-8"))
        for relative, nested, replacement, expected in cases:
            for locked_file in locked:
                shutil.copy2(ROOT / locked_file, repo / locked_file)
            schema_path = repo / relative
            schema = json.loads(schema_path.read_text(encoding="utf-8"))
            set_nested(schema, nested, replacement)
            schema_path.write_text(json.dumps(schema, indent=2) + "\n", encoding="utf-8")
            changed_lock = json.loads(json.dumps(baseline_lock))
            update_lock_hash(changed_lock, relative, schema_path)
            write_lock(lock_path, changed_lock)
            assert_contract_failure(repo, expected, relative)


def runtime_cases() -> tuple[tuple[str, tuple[str, ...], object, str], ...]:
    return (
        ("properties", ("properties",), [], "$.properties must be an object"),
        ("property schema", ("properties", "issue"), [], "$.properties.issue must be a boolean or object"),
        ("items", ("properties", "open_prs", "items"), [], "$.properties.open_prs.items must be a boolean or object"),
        ("additionalProperties", ("additionalProperties",), [], "$.additionalProperties must be a boolean or object"),
        ("required shape", ("required",), ["issue", 7], "$.required must be a unique array of strings"),
        ("unknown keyword", ("properties", "issue", "minimun"), 1, "$.properties.issue: unsupported JSON Schema keyword 'minimun'"),
        ("unknown type", ("properties", "issue", "type"), "uint64", "$.properties.issue.type must be a supported JSON type"),
        ("empty type array", ("properties", "issue", "type"), [], "$.properties.issue.type must be a supported JSON type"),
        ("duplicate type array", ("properties", "issue", "type"), ["integer", "integer"], "$.properties.issue.type must be a supported JSON type"),
        ("unknown union member", ("properties", "issue", "type"), ["integer", "uint64"], "$.properties.issue.type must be a supported JSON type"),
        ("enum shape", ("properties", "issue", "enum"), {"one": 1}, "$.properties.issue.enum must be a non-empty array"),
        ("empty enum", ("properties", "issue", "enum"), [], "$.properties.issue.enum must be a non-empty array"),
        ("enum duplicate", ("properties", "issue", "enum"), [1, 1.0], "$.properties.issue.enum must contain unique JSON values"),
        ("minLength bool", ("properties", "collected_at", "minLength"), True, "$.properties.collected_at.minLength must be a non-negative integer"),
        ("minItems negative", ("properties", "open_prs", "minItems"), -1, "$.properties.open_prs.minItems must be a non-negative integer"),
        ("minimum bool", ("properties", "issue", "minimum"), True, "$.properties.issue.minimum must be a JSON number"),
        ("minimum shape", ("properties", "issue", "minimum"), "one", "$.properties.issue.minimum must be a JSON number"),
        ("exclusiveMinimum shape", ("properties", "issue", "exclusiveMinimum"), "one", "$.properties.issue.exclusiveMinimum must be a JSON number"),
        ("exclusiveMaximum shape", ("properties", "issue", "exclusiveMaximum"), "ten", "$.properties.issue.exclusiveMaximum must be a JSON number"),
        ("minLength type compatibility", ("properties", "collected_at", "type"), "integer", "$.properties.collected_at.minLength requires only type string"),
        ("minLength missing type", ("properties", "collected_at"), {"minLength": 1}, "$.properties.collected_at.minLength requires explicit type string"),
        ("minLength unsafe union", ("properties", "collected_at", "type"), ["string", "null"], "$.properties.collected_at.minLength requires only type string"),
        ("items type compatibility", ("properties", "open_prs", "type"), "object", "$.properties.open_prs.items requires only type array"),
        ("minItems type compatibility", ("properties", "open_prs"), {"type": "object", "minItems": 1}, "$.properties.open_prs.minItems requires only type array"),
        ("minimum type compatibility", ("properties", "issue", "type"), "string", "$.properties.issue.minimum requires only type integer or number"),
        ("required type compatibility", ("properties", "open_prs", "items"), {"type": "array", "required": ["number"]}, "$.properties.open_prs.items.required requires only type object"),
        ("recursive item keyword", ("properties", "open_prs", "items", "minimun"), 1, "$.properties.open_prs.items: unsupported JSON Schema keyword 'minimun'"),
        ("recursive additional keyword", ("additionalProperties",), {"type": "string", "minimun": 1}, "$.additionalProperties: unsupported JSON Schema keyword 'minimun'"),
        ("boolean runtime child", ("properties", "issue"), True, "$.properties.issue boolean schema is not executable"),
        ("Python-only runtime regex", ("properties", "collected_at", "pattern"), "(?P<name>.+)", "invalid ECMAScript regex"),
        ("ECMAScript-only runtime regex", ("properties", "collected_at", "pattern"), "(?<name>.+)", "invalid Python runtime regex"),
    )


def assert_runtime_profile_matrix() -> None:
    runtime_accepts = {
        "duplicate type array", "minLength bool", "minItems negative", "minimum bool",
        "minLength missing type", "minLength unsafe union", "recursive additional keyword",
        "unknown union member", "enum duplicate",
    }
    with tempfile.TemporaryDirectory(prefix="remem-runtime-schema-") as raw:
        repo = Path(raw)
        copy_pack(repo)
        schema_path = repo / "schemas" / "duplicate_work_evidence.schema.json"
        lock_path = repo / "checks" / "specrail-sync.lock.json"
        baseline_schema = json.loads(schema_path.read_text(encoding="utf-8"))
        baseline_lock = json.loads(lock_path.read_text(encoding="utf-8"))

        open_path = repo / "schemas" / "open_required.schema.json"
        open_path.write_text(json.dumps({
            "$schema": "https://json-schema.org/draft/2020-12/schema", "title": "Open",
            "type": "object", "required": ["opaque"], "additionalProperties": True,
        }, indent=2) + "\n", encoding="utf-8")
        assert_passed(run_workflow(repo), "open required workflow")
        assert_passed(run_runtime(repo, open_path, {"opaque": 1}), "open required runtime")

        safe = json.loads(json.dumps(baseline_schema))
        safe["properties"]["issue"] = {"type": ["integer", "number"], "minimum": 1}
        schema_path.write_text(json.dumps(safe, indent=2) + "\n", encoding="utf-8")
        safe_lock = json.loads(json.dumps(baseline_lock))
        update_lock_hash(safe_lock, "schemas/duplicate_work_evidence.schema.json", schema_path)
        write_lock(lock_path, safe_lock)
        assert_passed(run_runtime(repo, schema_path), "safe numeric union runtime")
        assert_passed(run_workflow(repo), "safe numeric union workflow")
        assert_passed(
            run([str(repo / "scripts" / "sync-specrail-checks.sh"), "--verify"], cwd=repo),
            "safe numeric union sync",
        )

        const_schema = json.loads(json.dumps(baseline_schema))
        const_schema["properties"]["issue"]["const"] = 999
        schema_path.write_text(json.dumps(const_schema, indent=2) + "\n", encoding="utf-8")
        const_lock = json.loads(json.dumps(baseline_lock))
        update_lock_hash(const_lock, "schemas/duplicate_work_evidence.schema.json", schema_path)
        write_lock(lock_path, const_lock)
        assert run_runtime(repo, schema_path).returncode != 0
        assert_passed(run_workflow(repo), "ordinary const mismatch workflow")
        assert_passed(
            run([str(repo / "scripts" / "sync-specrail-checks.sh"), "--verify"], cwd=repo),
            "ordinary const mismatch sync",
        )

        for label, nested, replacement, expected in runtime_cases():
            malformed = json.loads(json.dumps(baseline_schema))
            set_nested(malformed, nested, replacement)
            schema_path.write_text(json.dumps(malformed, indent=2) + "\n", encoding="utf-8")
            changed_lock = json.loads(json.dumps(baseline_lock))
            update_lock_hash(changed_lock, "schemas/duplicate_work_evidence.schema.json", schema_path)
            write_lock(lock_path, changed_lock)
            runtime = run_runtime(repo, schema_path)
            if label in runtime_accepts:
                assert_passed(runtime, f"runtime contrast {label}")
            else:
                assert runtime.returncode != 0, f"runtime must reject {label}"
            assert_contract_failure(repo, expected, label)


def run_schema_contract_tests() -> None:
    assert_vocabulary_and_baselines()
    assert_regex_contracts()
    assert_boolean_children_and_enum_uniqueness()
    assert_published_contracts()
    assert_locked_schema_contracts()
    assert_runtime_profile_matrix()


def main() -> int:
    run_schema_contract_tests()
    print("Schema contract test passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
