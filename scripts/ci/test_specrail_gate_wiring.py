#!/usr/bin/env python3
"""Exercise the baseline SpecRail sync and workflow verification wiring."""

from __future__ import annotations

import hashlib
import json
import shutil
import sys
import tempfile
from pathlib import Path

from test_schema_contract import (
    assert_passed,
    copy_pack,
    run,
    run_schema_contract_tests,
    write_lock,
)


ROOT = Path(__file__).resolve().parents[2]
SYNC_SCRIPT = ROOT / "scripts" / "sync-specrail-checks.sh"
WORKFLOW_CHECK = ROOT / "checks" / "check_workflow.py"


def assert_runtime_verifier() -> None:
    with tempfile.TemporaryDirectory(prefix="remem-specrail-wiring-") as raw:
        repo = Path(raw)
        copy_pack(repo)
        sync_script = repo / "scripts" / "sync-specrail-checks.sh"
        lock_path = repo / "checks" / "specrail-sync.lock.json"
        baseline_lock = json.loads(lock_path.read_text(encoding="utf-8"))

        baseline = run([str(sync_script), "--verify"], cwd=repo)
        assert_passed(baseline, "isolated sync verifier baseline")
        assert "upstream-managed Python files classified" in baseline.stdout
        assert "local-owned excluded Python files classified" in baseline.stdout
        assert "classified SpecRail Python import closure" in baseline.stdout
        assert "SpecRail check passed" in baseline.stdout

        mismatched_lock = json.loads(json.dumps(baseline_lock))
        mismatched_lock["files"] = list(reversed(mismatched_lock["files"]))
        write_lock(lock_path, mismatched_lock)
        mismatched = run([str(sync_script), "--verify"], cwd=repo)
        assert mismatched.returncode != 0, "script/lock managed file mismatch must fail"
        assert "managed file list does not match lock" in mismatched.stdout

        mismatched_excluded = json.loads(json.dumps(baseline_lock))
        mismatched_excluded["excluded"] = list(reversed(mismatched_excluded["excluded"]))
        write_lock(lock_path, mismatched_excluded)
        excluded = run([str(sync_script), "--verify"], cwd=repo)
        assert excluded.returncode != 0, "script/lock local-owned mismatch must fail"
        assert "local-owned excluded file list does not match lock" in excluded.stdout

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
        assert missing_managed.returncode != 0
        assert "files match lock" in missing_managed.stdout
        assert "specrail_missing_managed_dependency" in missing_managed.stderr

        shutil.copy2(ROOT / "checks" / "github_evidence_common.py", broken_managed)
        write_lock(lock_path, baseline_lock)
        helper_mutations = (
            ("bare", "import specrail_untracked_helper", "checks/specrail_untracked_helper.py", "UNCLASSIFIED LOCAL IMPORT"),
            ("qualified", "import checks.specrail_untracked_helper", "checks/specrail_untracked_helper.py", "UNCLASSIFIED LOCAL IMPORT"),
            ("from checks multi-name", "from checks import specrail_lib, specrail_untracked_helper", "checks/specrail_untracked_helper.py", "UNCLASSIFIED LOCAL IMPORT"),
            ("nested qualified", "import checks.specrail_untracked.specrail_helper", "checks/specrail_untracked/specrail_helper.py", "UNCLASSIFIED LOCAL IMPORT"),
            ("relative", "from . import specrail_lib", None, "UNSUPPORTED RELATIVE LOCAL IMPORT"),
            ("path escape", "import checks.specrail_escape_helper", None, "LOCAL IMPORT PATH ESCAPE"),
        )
        side_effect = repo / "untrusted-helper-executed"
        for label, statement, helper_relative, expected in helper_mutations:
            shutil.copy2(ROOT / "checks" / "github_evidence_common.py", broken_managed)
            if helper_relative:
                helper_path = repo / helper_relative
                helper_path.parent.mkdir(parents=True, exist_ok=True)
                helper_path.write_text(
                    "from pathlib import Path\n"
                    "Path('untrusted-helper-executed').write_text('bad')\n",
                    encoding="utf-8",
                )
            elif label == "path escape":
                outside_helper = repo / "outside_helper.py"
                outside_helper.write_text(
                    "from pathlib import Path\n"
                    "Path('untrusted-helper-executed').write_text('bad')\n",
                    encoding="utf-8",
                )
                (repo / "checks" / "specrail_escape_helper.py").symlink_to(
                    outside_helper
                )
            broken_managed.write_text(
                f"if False:\n    {statement}\n"
                + broken_managed.read_text(encoding="utf-8"),
                encoding="utf-8",
            )
            helper_lock = json.loads(json.dumps(baseline_lock))
            for entry in helper_lock["files"]:
                if entry["path"] == "checks/github_evidence_common.py":
                    entry["sha256"] = hashlib.sha256(broken_managed.read_bytes()).hexdigest()
                    break
            write_lock(lock_path, helper_lock)
            unclassified_import = run([str(sync_script), "--verify"], cwd=repo)
            assert unclassified_import.returncode != 0, f"{label} import must fail"
            assert "files match lock" in unclassified_import.stdout
            assert expected in unclassified_import.stderr
            if helper_relative:
                assert helper_relative in unclassified_import.stderr
            elif label == "path escape":
                assert "checks/specrail_escape_helper.py" in unclassified_import.stderr
            assert not side_effect.exists(), f"{label} helper must not execute"
            top_helper = repo / "checks" / "specrail_untracked_helper.py"
            if top_helper.exists():
                top_helper.unlink()
            nested_helper = repo / "checks" / "specrail_untracked"
            if nested_helper.exists():
                shutil.rmtree(nested_helper)
            escape_helper = repo / "checks" / "specrail_escape_helper.py"
            if escape_helper.exists():
                escape_helper.unlink()
            outside_helper = repo / "outside_helper.py"
            if outside_helper.exists():
                outside_helper.unlink()

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
        assert missing_workflow.returncode != 0
        assert "files match lock" in missing_workflow.stdout
        assert "IMPORT FAILED: checks/check_workflow.py" in missing_workflow.stderr
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
    run_schema_contract_tests()
    assert_runtime_verifier()
    print("SpecRail gate wiring test passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
