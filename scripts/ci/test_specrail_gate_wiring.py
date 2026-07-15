#!/usr/bin/env python3
"""Exercise the baseline SpecRail sync and workflow verification wiring."""

from __future__ import annotations

import hashlib
import json
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
SYNC_SCRIPT = ROOT / "scripts" / "sync-specrail-checks.sh"
WORKFLOW_CHECK = ROOT / "checks" / "check_workflow.py"
PACK_DIRS = ("checks", "policies", "review", "schemas", "skills", "templates", "tools")
PACK_FILES = (
    "AGENT_USAGE.md",
    "AGENTS.md",
    "labels.yaml",
    "skills-lock.json",
    "states.yaml",
    "workflow.yaml",
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


def run(command: list[str], *, cwd: Path) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        command,
        cwd=cwd,
        capture_output=True,
        text=True,
        check=False,
    )


def assert_passed(completed: subprocess.CompletedProcess[str], label: str) -> None:
    assert completed.returncode == 0, (
        f"{label} failed:\n{completed.stdout}{completed.stderr}"
    )


def copy_pack(repo: Path) -> None:
    for relative_path in PACK_DIRS:
        shutil.copytree(
            ROOT / relative_path,
            repo / relative_path,
            ignore=shutil.ignore_patterns("__pycache__", "*.pyc"),
        )
    for relative_path in PACK_FILES:
        shutil.copy2(ROOT / relative_path, repo / relative_path)
    target_script = repo / "scripts" / "sync-specrail-checks.sh"
    target_script.parent.mkdir(parents=True)
    shutil.copy2(SYNC_SCRIPT, target_script)


def write_lock(lock_path: Path, lock: dict[str, object]) -> None:
    lock_path.write_text(json.dumps(lock, indent=2) + "\n", encoding="utf-8")


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
        assert isinstance(child, dict), f"test fixture path {path!r} is not an object"
        target = child
    target[path[-1]] = replacement


def run_runtime_schema_validation(
    repo: Path,
    schema_path: Path,
    data: dict[str, object],
) -> subprocess.CompletedProcess[str]:
    return run(
        [
            sys.executable,
            "-c",
            RUNTIME_SCHEMA_SCRIPT,
            str(repo / "checks"),
            str(schema_path),
            json.dumps(data),
        ],
        cwd=repo,
    )


def assert_malformed_schema_bodies_fail_closed() -> None:
    cases = (
        ("properties", ("properties",), [], "$.properties must be an object"),
        (
            "property schema",
            ("properties", "issue"),
            [],
            "$.properties.issue must be an object",
        ),
        (
            "items",
            ("properties", "open_prs", "items"),
            [],
            "$.properties.open_prs.items must be an object",
        ),
        (
            "additionalProperties",
            ("additionalProperties",),
            [],
            "$.additionalProperties must be a boolean or object",
        ),
        (
            "required shape",
            ("required",),
            ["issue", 7],
            "$.required must be an array of strings",
        ),
        (
            "unknown keyword",
            ("properties", "issue", "minimun"),
            1,
            "$.properties.issue: unsupported JSON Schema keyword 'minimun'",
        ),
        (
            "unknown type",
            ("properties", "issue", "type"),
            "uint64",
            "$.properties.issue.type must be a supported JSON type",
        ),
        (
            "empty type array",
            ("properties", "issue", "type"),
            [],
            "$.properties.issue.type must be a supported JSON type",
        ),
        (
            "enum shape",
            ("properties", "issue", "enum"),
            {"one": 1},
            "$.properties.issue.enum must be a non-empty array",
        ),
        (
            "empty enum",
            ("properties", "issue", "enum"),
            [],
            "$.properties.issue.enum must be a non-empty array",
        ),
        (
            "minLength bool",
            ("properties", "collected_at", "minLength"),
            True,
            "$.properties.collected_at.minLength must be a non-negative integer",
        ),
        (
            "minItems negative",
            ("properties", "open_prs", "minItems"),
            -1,
            "$.properties.open_prs.minItems must be a non-negative integer",
        ),
        (
            "minimum bool",
            ("properties", "issue", "minimum"),
            True,
            "$.properties.issue.minimum must be a JSON number",
        ),
        (
            "minimum shape",
            ("properties", "issue", "minimum"),
            "one",
            "$.properties.issue.minimum must be a JSON number",
        ),
        (
            "exclusiveMinimum shape",
            ("properties", "issue", "exclusiveMinimum"),
            "one",
            "$.properties.issue.exclusiveMinimum must be a JSON number",
        ),
        (
            "exclusiveMaximum shape",
            ("properties", "issue", "exclusiveMaximum"),
            "ten",
            "$.properties.issue.exclusiveMaximum must be a JSON number",
        ),
        (
            "minLength type compatibility",
            ("properties", "collected_at", "type"),
            "integer",
            "$.properties.collected_at.minLength requires type string",
        ),
        (
            "items type compatibility",
            ("properties", "open_prs", "type"),
            "object",
            "$.properties.open_prs.items requires type array",
        ),
        (
            "minItems type compatibility",
            ("properties", "open_prs"),
            {"type": "object", "minItems": 1},
            "$.properties.open_prs.minItems requires type array",
        ),
        (
            "minimum type compatibility",
            ("properties", "issue", "type"),
            "string",
            "$.properties.issue.minimum requires type integer or number",
        ),
        (
            "required type compatibility",
            ("properties", "open_prs", "items"),
            {"type": "array", "required": ["number"]},
            "$.properties.open_prs.items.required requires type object",
        ),
        (
            "properties type compatibility",
            ("properties", "open_prs", "items"),
            {"type": "array", "properties": {}},
            "$.properties.open_prs.items.properties requires type object",
        ),
        (
            "additionalProperties type compatibility",
            ("properties", "open_prs", "items"),
            {"type": "array", "additionalProperties": False},
            "$.properties.open_prs.items.additionalProperties requires type object",
        ),
    )
    runtime_accepts_malformed = {"minLength bool", "minItems negative", "minimum bool"}
    runtime_data = {
        "issue": 1,
        "collected_at": "now",
        "open_prs_complete": True,
        "open_pr_limit": 10,
        "open_prs": [
            {"number": 2, "head_ref": "review-fix", "references_issue": True}
        ],
        "remote_branches": [],
    }

    with tempfile.TemporaryDirectory(prefix="remem-specrail-schema-body-") as raw:
        repo = Path(raw)
        copy_pack(repo)
        schema_path = repo / "schemas" / "duplicate_work_evidence.schema.json"
        lock_path = repo / "checks" / "specrail-sync.lock.json"
        baseline_schema = json.loads(schema_path.read_text(encoding="utf-8"))
        baseline_lock = json.loads(lock_path.read_text(encoding="utf-8"))

        open_schema_path = repo / "schemas" / "open_required.schema.json"
        open_schema_path.write_text(
            json.dumps(
                {
                    "$schema": "https://json-schema.org/draft/2020-12/schema",
                    "title": "Open required object",
                    "type": "object",
                    "required": ["opaque"],
                    "additionalProperties": True,
                },
                indent=2,
            )
            + "\n",
            encoding="utf-8",
        )
        open_workflow = run(
            [
                sys.executable,
                str(repo / "checks" / "check_workflow.py"),
                "--repo",
                str(repo),
            ],
            cwd=repo,
        )
        assert_passed(open_workflow, "open object required property workflow check")
        assert_passed(
            run_runtime_schema_validation(repo, open_schema_path, {"opaque": 1}),
            "open object required property runtime validation",
        )

        compatible_schema = json.loads(json.dumps(baseline_schema))
        del compatible_schema["properties"]["collected_at"]["type"]
        compatible_schema["properties"]["issue"]["type"] = ["null", "integer"]
        compatible_schema["properties"]["open_prs"]["type"] = ["null", "array"]
        compatible_schema["properties"]["open_prs"]["items"]["type"] = [
            "null",
            "object",
        ]
        schema_path.write_text(
            json.dumps(compatible_schema, indent=2) + "\n",
            encoding="utf-8",
        )
        compatible_lock = json.loads(json.dumps(baseline_lock))
        update_lock_hash(
            compatible_lock,
            "schemas/duplicate_work_evidence.schema.json",
            schema_path,
        )
        write_lock(lock_path, compatible_lock)
        assert_passed(
            run_runtime_schema_validation(repo, schema_path, runtime_data),
            "missing and union runtime schema types",
        )
        assert_passed(
            run(
                [
                    sys.executable,
                    str(repo / "checks" / "check_workflow.py"),
                    "--repo",
                    str(repo),
                ],
                cwd=repo,
            ),
            "missing and union workflow schema types",
        )
        assert_passed(
            run([str(repo / "scripts" / "sync-specrail-checks.sh"), "--verify"], cwd=repo),
            "missing and union sync schema types",
        )

        for label, path, replacement, expected_error in cases:
            malformed = json.loads(json.dumps(baseline_schema))
            set_nested(malformed, path, replacement)
            schema_path.write_text(
                json.dumps(malformed, indent=2) + "\n",
                encoding="utf-8",
            )
            malformed_lock = json.loads(json.dumps(baseline_lock))
            update_lock_hash(
                malformed_lock,
                "schemas/duplicate_work_evidence.schema.json",
                schema_path,
            )
            write_lock(lock_path, malformed_lock)

            runtime_check = run_runtime_schema_validation(repo, schema_path, runtime_data)
            if label in runtime_accepts_malformed:
                assert_passed(runtime_check, f"runtime malformed {label} contrast")
            else:
                assert runtime_check.returncode != 0, (
                    f"runtime must reject malformed {label}"
                )

            workflow_check = run(
                [
                    sys.executable,
                    str(repo / "checks" / "check_workflow.py"),
                    "--repo",
                    str(repo),
                ],
                cwd=repo,
            )
            assert workflow_check.returncode != 0, (
                f"malformed {label} must fail the workflow check"
            )
            assert expected_error in workflow_check.stdout

            sync_verify = run(
                [str(repo / "scripts" / "sync-specrail-checks.sh"), "--verify"],
                cwd=repo,
            )
            assert sync_verify.returncode != 0, (
                f"malformed {label} must fail sync verification"
            )
            assert "files match lock" in sync_verify.stdout
            assert expected_error in sync_verify.stdout


def assert_runtime_verifier() -> None:
    with tempfile.TemporaryDirectory(prefix="remem-specrail-wiring-") as raw:
        repo = Path(raw)
        copy_pack(repo)
        sync_script = repo / "scripts" / "sync-specrail-checks.sh"
        lock_path = repo / "checks" / "specrail-sync.lock.json"
        baseline_lock = json.loads(lock_path.read_text(encoding="utf-8"))

        baseline = run([str(sync_script), "--verify"], cwd=repo)
        assert_passed(baseline, "isolated sync verifier baseline")
        assert "managed SpecRail Python import closure" in baseline.stdout
        assert "SpecRail check passed" in baseline.stdout

        mismatched_lock = json.loads(json.dumps(baseline_lock))
        mismatched_lock["files"] = list(reversed(mismatched_lock["files"]))
        write_lock(lock_path, mismatched_lock)
        mismatched = run([str(sync_script), "--verify"], cwd=repo)
        assert mismatched.returncode != 0, "script/lock managed file mismatch must fail"
        assert "managed file list does not match lock" in mismatched.stdout

        write_lock(lock_path, baseline_lock)
        broken_managed = repo / "checks" / "github_evidence_common.py"
        broken_managed.write_text(
            "import specrail_missing_managed_dependency\n"
            + broken_managed.read_text(encoding="utf-8"),
            encoding="utf-8",
        )
        managed_lock = json.loads(json.dumps(baseline_lock))
        for entry in managed_lock["files"]:
            if entry["path"] == "checks/github_evidence_common.py":
                entry["sha256"] = hashlib.sha256(broken_managed.read_bytes()).hexdigest()
                break
        write_lock(lock_path, managed_lock)
        missing_managed = run([str(sync_script), "--verify"], cwd=repo)
        assert missing_managed.returncode != 0, (
            "missing managed import must fail even after its lock hash is updated"
        )
        assert "files match lock" in missing_managed.stdout
        assert "specrail_missing_managed_dependency" in missing_managed.stderr

        shutil.copy2(ROOT / "checks" / "github_evidence_common.py", broken_managed)
        write_lock(lock_path, baseline_lock)
        broken_workflow = repo / "checks" / "check_workflow.py"
        broken_workflow.write_text(
            broken_workflow.read_text(encoding="utf-8").replace(
                "import argparse\n",
                "import specrail_missing_workflow_dependency\nimport argparse\n",
                1,
            ),
            encoding="utf-8",
        )
        missing_workflow = run([str(sync_script), "--verify"], cwd=repo)
        assert missing_workflow.returncode != 0, (
            "excluded workflow checker runtime failures must fail sync verification"
        )
        assert "files match lock" in missing_workflow.stdout
        assert "managed SpecRail Python import closure" in missing_workflow.stdout
        assert "specrail_missing_workflow_dependency" in missing_workflow.stderr


def main() -> int:
    assert_passed(
        run([sys.executable, str(WORKFLOW_CHECK), "--repo", str(ROOT)], cwd=ROOT),
        "repository workflow check",
    )
    assert_passed(
        run([str(SYNC_SCRIPT), "--verify"], cwd=ROOT),
        "repository sync verifier",
    )
    assert_malformed_schema_bodies_fail_closed()
    assert_runtime_verifier()
    print("SpecRail gate wiring test passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
