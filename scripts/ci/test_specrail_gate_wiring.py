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
    assert_runtime_verifier()
    print("SpecRail gate wiring test passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
