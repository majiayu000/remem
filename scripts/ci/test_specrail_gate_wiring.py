#!/usr/bin/env python3
"""Exercise the baseline SpecRail sync and workflow verification wiring."""

from __future__ import annotations

import hashlib
import json
import os
import py_compile
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
            ("dynamic importlib literal", "import importlib; importlib.import_module('specrail_untracked_helper')", "checks/specrail_untracked_helper.py", "UNCLASSIFIED LOCAL IMPORT"),
            ("dynamic builtin literal", "__import__('specrail_untracked_helper')", "checks/specrail_untracked_helper.py", "UNCLASSIFIED LOCAL IMPORT"),
            ("dynamic nonliteral", "import importlib; module_name = 'specrail_untracked_helper'; importlib.import_module(module_name)", "checks/specrail_untracked_helper.py", "NON-LITERAL DYNAMIC IMPORT"),
            ("builtins attribute literal", "import builtins; builtins.__import__('specrail_untracked_helper')", "checks/specrail_untracked_helper.py", "UNCLASSIFIED LOCAL IMPORT"),
            ("builtins aliased attribute literal", "import builtins as builtin_api; builtin_api.__import__('specrail_untracked_helper')", "checks/specrail_untracked_helper.py", "UNCLASSIFIED LOCAL IMPORT"),
            ("builtins imported alias literal", "from builtins import __import__ as dyn_import; dyn_import('specrail_untracked_helper')", "checks/specrail_untracked_helper.py", "UNCLASSIFIED LOCAL IMPORT"),
            ("builtins attribute nonliteral", "import builtins; module_name = 'specrail_untracked_helper'; builtins.__import__(module_name)", "checks/specrail_untracked_helper.py", "NON-LITERAL DYNAMIC IMPORT"),
            ("builtins imported alias nonliteral", "from builtins import __import__ as dyn_import; module_name = 'specrail_untracked_helper'; dyn_import(module_name)", "checks/specrail_untracked_helper.py", "NON-LITERAL DYNAMIC IMPORT"),
            ("fromlist keyword literal", "__import__('checks', fromlist=['specrail_untracked_helper'])", "checks/specrail_untracked_helper.py", "UNCLASSIFIED LOCAL IMPORT"),
            ("fromlist positional literal", "__import__('checks', None, None, ['specrail_untracked_helper'])", "checks/specrail_untracked_helper.py", "UNCLASSIFIED LOCAL IMPORT"),
            ("fromlist nonliteral list", "names = ['specrail_untracked_helper']; __import__('checks', fromlist=names)", "checks/specrail_untracked_helper.py", "NON-LITERAL DYNAMIC IMPORT"),
            ("fromlist nonliteral entry", "name = 'specrail_untracked_helper'; __import__('checks', fromlist=[name])", "checks/specrail_untracked_helper.py", "NON-LITERAL DYNAMIC IMPORT"),
            ("fromlist wildcard", "__import__('checks', fromlist=['*'])", "checks/specrail_untracked_helper.py", "NON-LITERAL DYNAMIC IMPORT"),
            ("dynamic nonliteral level", "level = 0; __import__('specrail_lib', level=level)", "checks/specrail_untracked_helper.py", "NON-LITERAL DYNAMIC IMPORT"),
            ("dynamic relative level", "__import__('specrail_lib', level=1)", "checks/specrail_untracked_helper.py", "UNSUPPORTED RELATIVE LOCAL IMPORT"),
            ("assigned import_module alias", "import importlib; loader = importlib.import_module; loader('specrail_untracked_helper')", "checks/specrail_untracked_helper.py", "DYNAMIC IMPORT ALIAS"),
            ("assigned builtin import alias", "loader = __import__; loader('specrail_untracked_helper')", "checks/specrail_untracked_helper.py", "DYNAMIC IMPORT ALIAS"),
            ("aliased importlib module", "import importlib; il = importlib; il.import_module('specrail_untracked_helper')", "checks/specrail_untracked_helper.py", "DYNAMIC IMPORT ALIAS"),
            ("getattr importlib import", "import importlib; getattr(importlib, 'import_module')('specrail_untracked_helper')", "checks/specrail_untracked_helper.py", "DYNAMIC IMPORT ALIAS"),
            ("foreign import_module attribute", "import checks_loader_stub as stub; stub.import_module('specrail_untracked_helper')", "checks/specrail_untracked_helper.py", "DYNAMIC IMPORT ALIAS"),
            ("importlib util submodule", "import importlib.util as loader_util; spec = loader_util.spec_from_file_location('specrail_untracked_helper', 'checks/specrail_untracked_helper.py'); module = loader_util.module_from_spec(spec); spec.loader.exec_module(module)", "checks/specrail_untracked_helper.py", "UNSUPPORTED IMPORTLIB LOADER SURFACE"),
            ("importlib named util", "from importlib import util as loader_util; spec = loader_util.spec_from_file_location('specrail_untracked_helper', 'checks/specrail_untracked_helper.py'); module = loader_util.module_from_spec(spec); spec.loader.exec_module(module)", "checks/specrail_untracked_helper.py", "UNSUPPORTED IMPORTLIB LOADER SURFACE"),
            ("source file loader alias", "from importlib.machinery import SourceFileLoader as Loader; Loader('specrail_untracked_helper', 'checks/specrail_untracked_helper.py').load_module()", "checks/specrail_untracked_helper.py", "UNSUPPORTED IMPORTLIB LOADER SURFACE"),
            ("dynamic importlib util loader", "import importlib; loader_util = importlib.import_module('importlib.util'); spec = loader_util.spec_from_file_location('specrail_untracked_helper', 'checks/specrail_untracked_helper.py'); module = loader_util.module_from_spec(spec); spec.loader.exec_module(module)", "checks/specrail_untracked_helper.py", "UNSUPPORTED IMPORTLIB LOADER SURFACE"),
            ("dynamic builtin importlib loader", "Loader = __import__('importlib.machinery', fromlist=['SourceFileLoader']).SourceFileLoader; Loader('specrail_untracked_helper', 'checks/specrail_untracked_helper.py').load_module()", "checks/specrail_untracked_helper.py", "UNSUPPORTED IMPORTLIB LOADER SURFACE"),
            ("sys modules loader", "import sys; sys.modules['importlib.machinery'].SourceFileLoader('specrail_untracked_helper', 'checks/specrail_untracked_helper.py').load_module()", "checks/specrail_untracked_helper.py", "UNSUPPORTED IMPORTLIB LOADER SURFACE"),
            ("exec file contents", "from pathlib import Path; exec(Path('checks/specrail_untracked_helper.py').read_text())", "checks/specrail_untracked_helper.py", "UNSUPPORTED DYNAMIC CODE EXECUTION"),
            ("eval file contents", "from pathlib import Path; eval(Path('checks/specrail_untracked_helper.py').read_text().splitlines()[-1])", "checks/specrail_untracked_helper.py", "UNSUPPORTED DYNAMIC CODE EXECUTION"),
            ("import-only exec alias", "from builtins import exec as run_code", None, "UNSUPPORTED DYNAMIC CODE EXECUTION"),
            ("import-only eval", "from builtins import eval", None, "UNSUPPORTED DYNAMIC CODE EXECUTION"),
            ("imported exec alias", "from builtins import exec as run_code; from pathlib import Path; run_code(Path('checks/specrail_untracked_helper.py').read_text())", "checks/specrail_untracked_helper.py", "UNSUPPORTED DYNAMIC CODE EXECUTION"),
            ("builtins eval attribute", "import builtins as builtin_api; from pathlib import Path; builtin_api.eval(Path('checks/specrail_untracked_helper.py').read_text().splitlines()[-1])", "checks/specrail_untracked_helper.py", "UNSUPPORTED DYNAMIC CODE EXECUTION"),
            ("dynamic builtins exec", "from pathlib import Path; __import__('builtins').exec(Path('checks/specrail_untracked_helper.py').read_text())", "checks/specrail_untracked_helper.py", "UNSUPPORTED DYNAMIC CODE EXECUTION"),
            ("dunder builtins exec", "from pathlib import Path; __builtins__['exec'](Path('checks/specrail_untracked_helper.py').read_text())", "checks/specrail_untracked_helper.py", "UNSUPPORTED DYNAMIC CODE EXECUTION"),
            ("imported builtins dictionary", "from builtins import __dict__ as bdict; from pathlib import Path; bdict['exec'](Path('checks/specrail_untracked_helper.py').read_text())", "checks/specrail_untracked_helper.py", "UNSUPPORTED DYNAMIC CODE EXECUTION"),
            ("sys path insert", "import sys; sys.path.insert(0, 'tools'); import specrail_untrusted_helper", "tools/specrail_untrusted_helper.py", "UNSUPPORTED SYS PATH ACCESS"),
            ("sys path assignment", "import sys; sys.path = ['tools'] + sys.path", None, "UNSUPPORTED SYS PATH ACCESS"),
            ("from sys import path alias", "from sys import path as search_path; search_path.append('tools')", None, "UNSUPPORTED SYS PATH ACCESS"),
            ("sys star import", "from sys import *", None, "UNSUPPORTED SYS PATH ACCESS"),
            ("importlib star import", "from importlib import *; import_module('specrail_untracked_helper')", "checks/specrail_untracked_helper.py", "UNSUPPORTED IMPORTLIB LOADER SURFACE"),
            ("builtins star import", "from builtins import *", None, "DYNAMIC IMPORT ALIAS"),
            ("sys alias assignment", "import sys; s = sys; getattr(s, 'path').insert(0, 'tools')", None, "UNSUPPORTED SYS PATH ACCESS"),
            ("sys import-alias getattr", "import sys as s; getattr(s, 'path').insert(0, 'tools')", None, "UNSUPPORTED SYS PATH ACCESS"),
            ("sourceless pyc helper", "import specrail_untracked_helper", None, "SOURCELESS LOCAL IMPORT"),
            ("outside checks absolute", "import tools.specrail_untrusted_helper", "tools/specrail_untrusted_helper.py", "UNCLASSIFIED LOCAL IMPORT"),
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
            elif label == "sourceless pyc helper":
                helper_source = repo / "specrail_sourceless_src.py"
                helper_source.write_text(
                    "from pathlib import Path\n"
                    "Path('untrusted-helper-executed').write_text('bad')\n",
                    encoding="utf-8",
                )
                py_compile.compile(
                    str(helper_source),
                    cfile=str(repo / "checks" / "specrail_untracked_helper.pyc"),
                )
                helper_source.unlink()
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
                f"{statement}\n"
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
            if expected in {
                "UNSUPPORTED IMPORTLIB LOADER SURFACE",
                "UNSUPPORTED DYNAMIC CODE EXECUTION",
            }:
                assert "checks/github_evidence_common.py" in unclassified_import.stderr
            if helper_relative and expected == "UNCLASSIFIED LOCAL IMPORT":
                assert helper_relative in unclassified_import.stderr
            elif label == "path escape":
                assert "checks/specrail_escape_helper.py" in unclassified_import.stderr
            assert not side_effect.exists(), f"{label} helper must not execute"
            top_helper = repo / "checks" / "specrail_untracked_helper.py"
            if top_helper.exists():
                top_helper.unlink()
            pyc_helper = repo / "checks" / "specrail_untracked_helper.pyc"
            if pyc_helper.exists():
                pyc_helper.unlink()
            nested_helper = repo / "checks" / "specrail_untracked"
            if nested_helper.exists():
                shutil.rmtree(nested_helper)
            tools_helper = repo / "tools" / "specrail_untrusted_helper.py"
            if tools_helper.exists():
                tools_helper.unlink()
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


def assert_sync_copy_allows_new_managed_file() -> None:
    with (
        tempfile.TemporaryDirectory(prefix="remem-sync-target-") as target_raw,
        tempfile.TemporaryDirectory(prefix="remem-sync-upstream-") as upstream_raw,
    ):
        target = Path(target_raw)
        upstream = Path(upstream_raw)
        copy_pack(target)
        copy_pack(upstream)
        new_managed = upstream / "checks" / "specrail_new_upstream.py"
        new_managed.write_text("VALUE = 1\n", encoding="utf-8")
        new_managed.chmod(0o755)
        new_schema = upstream / "schemas" / "specrail_new_upstream.schema.json"
        shutil.copy2(upstream / "schemas" / "review_result.schema.json", new_schema)
        assert_passed(run(["git", "add", "-A"], cwd=upstream), "stage upstream fixture")
        assert_passed(
            run(
                [
                    "git", "-c", "user.name=SpecRail Test",
                    "-c", "user.email=test@example.invalid",
                    "commit", "-qm", "add upstream check fixture",
                ],
                cwd=upstream,
            ),
            "commit upstream fixture",
        )

        sync_script = target / "scripts" / "sync-specrail-checks.sh"
        script = sync_script.read_text(encoding="utf-8")
        needle = '  "checks/specrail_lib.py"\n'
        assert needle in script
        script = script.replace(
                needle,
                needle + '  "checks/specrail_new_upstream.py"\n',
                1,
        )
        schema_needle = '  "schemas/runtime_checkpoint.schema.json"\n'
        assert schema_needle in script
        sync_script.write_text(
            script.replace(
                schema_needle,
                schema_needle + '  "schemas/specrail_new_upstream.schema.json"\n',
                1,
            ),
            encoding="utf-8",
        )
        index_before = run(["git", "ls-files", "--stage"], cwd=target)
        assert_passed(index_before, "read target index before sync")
        sync_copy = run([str(sync_script), str(upstream)], cwd=target)
        assert_passed(sync_copy, "normal sync with newly copied managed check")
        assert "2 newly copied upstream-managed files pending tracking" in sync_copy.stdout
        assert (target / "checks" / "specrail_new_upstream.py").is_file()
        assert os.access(target / "checks" / "specrail_new_upstream.py", os.X_OK), (
            "synced 100755 upstream check must stay executable"
        )
        assert not os.access(target / "checks" / "github_evidence_common.py", os.X_OK), (
            "synced 100644 upstream check must not gain the executable bit"
        )
        assert (target / "schemas" / "specrail_new_upstream.schema.json").is_file()
        index_after = run(["git", "ls-files", "--stage"], cwd=target)
        assert_passed(index_after, "read target index after sync")
        assert index_after.stdout == index_before.stdout, "write sync must not alter target index"

        strict_verify = run([str(sync_script), "--verify"], cwd=target)
        assert strict_verify.returncode != 0
        assert "CLASSIFIED FILE IS NOT TRACKED: checks/specrail_new_upstream.py" in strict_verify.stderr
        assert "CLASSIFIED FILE IS NOT TRACKED: schemas/specrail_new_upstream.schema.json" in strict_verify.stderr

        assert_passed(
            run(["git", "add", "checks/specrail_new_upstream.py"], cwd=target),
            "stage new managed Python fixture",
        )
        schema_untracked = run([str(sync_script), "--verify"], cwd=target)
        assert schema_untracked.returncode != 0
        assert "CLASSIFIED FILE IS NOT TRACKED: schemas/specrail_new_upstream.schema.json" in schema_untracked.stderr

        assert_passed(
            run(["git", "add", "schemas/specrail_new_upstream.schema.json"], cwd=target),
            "stage new managed schema fixture",
        )
        assert_passed(
            run([str(sync_script), "--verify"], cwd=target),
            "strict sync verify after tracking new managed files",
        )


def assert_sync_rejects_unindexed_previously_locked_file() -> None:
    with (
        tempfile.TemporaryDirectory(prefix="remem-sync-rmcached-target-") as target_raw,
        tempfile.TemporaryDirectory(prefix="remem-sync-rmcached-upstream-") as upstream_raw,
    ):
        target = Path(target_raw)
        upstream = Path(upstream_raw)
        copy_pack(target)
        copy_pack(upstream)
        assert_passed(
            run(
                [
                    "git", "-c", "user.name=SpecRail Test",
                    "-c", "user.email=test@example.invalid",
                    "commit", "-qm", "baseline upstream fixture",
                ],
                cwd=upstream,
            ),
            "commit rm-cached upstream baseline",
        )
        assert_passed(
            run(["git", "rm", "--cached", "checks/specrail_lib.py"], cwd=target),
            "drop previously locked managed file from target index",
        )
        rejected = run(
            [str(target / "scripts" / "sync-specrail-checks.sh"), str(upstream)],
            cwd=target,
        )
        assert rejected.returncode != 0, "sync must fail when a locked managed file left the index"
        assert "CLASSIFIED FILE IS NOT TRACKED: checks/specrail_lib.py" in rejected.stderr


def assert_upstream_source_preflight() -> None:
    cases = (
        ("untracked", "UPSTREAM HEAD DOES NOT TRACK"),
        ("dirty", "UPSTREAM WORKTREE DRIFT"),
        ("staged", "UPSTREAM INDEX DRIFT"),
        ("symlink", "UPSTREAM HEAD PATH IS NOT A REGULAR FILE"),
    )
    for mode, expected in cases:
        with (
            tempfile.TemporaryDirectory(prefix=f"remem-sync-{mode}-target-") as target_raw,
            tempfile.TemporaryDirectory(prefix=f"remem-sync-{mode}-upstream-") as upstream_raw,
        ):
            target = Path(target_raw)
            upstream = Path(upstream_raw)
            copy_pack(target)
            copy_pack(upstream)
            assert_passed(
                run(
                    [
                        "git", "-c", "user.name=SpecRail Test",
                        "-c", "user.email=test@example.invalid",
                        "commit", "-qm", "baseline upstream fixture",
                    ],
                    cwd=upstream,
                ),
                f"commit {mode} upstream baseline",
            )
            relative = "checks/github_evidence_common.py"
            upstream_file = upstream / relative
            if mode == "untracked":
                assert_passed(
                    run(["git", "rm", "--cached", relative], cwd=upstream),
                    "remove upstream fixture from index",
                )
                assert_passed(
                    run(
                        [
                            "git", "-c", "user.name=SpecRail Test",
                            "-c", "user.email=test@example.invalid",
                            "commit", "-qm", "remove upstream fixture",
                        ],
                        cwd=upstream,
                    ),
                    "commit upstream fixture removal",
                )
            elif mode == "symlink":
                payload = upstream / "specrail_symlink_payload.py"
                payload.write_text("VALUE = 1\n", encoding="utf-8")
                upstream_file.unlink()
                upstream_file.symlink_to("../specrail_symlink_payload.py")
                assert_passed(
                    run(["git", "add", "-A"], cwd=upstream),
                    "stage upstream symlink fixture",
                )
                assert_passed(
                    run(
                        [
                            "git", "-c", "user.name=SpecRail Test",
                            "-c", "user.email=test@example.invalid",
                            "commit", "-qm", "replace upstream fixture with symlink",
                        ],
                        cwd=upstream,
                    ),
                    "commit upstream symlink fixture",
                )
            else:
                upstream_file.write_text(
                    "# uncommitted upstream drift\n"
                    + upstream_file.read_text(encoding="utf-8"),
                    encoding="utf-8",
                )
                if mode == "staged":
                    assert_passed(
                        run(["git", "add", relative], cwd=upstream),
                        "stage upstream drift fixture",
                    )

            original_target = (target / relative).read_bytes()
            original_lock = (target / "checks" / "specrail-sync.lock.json").read_bytes()
            rejected = run(
                [str(target / "scripts" / "sync-specrail-checks.sh"), str(upstream)],
                cwd=target,
            )
            assert rejected.returncode != 0, f"{mode} upstream source must fail"
            assert expected in rejected.stderr
            assert (target / relative).read_bytes() == original_target
            assert (target / "checks" / "specrail-sync.lock.json").read_bytes() == original_lock


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
    assert_sync_copy_allows_new_managed_file()
    assert_sync_rejects_unindexed_previously_locked_file()
    assert_upstream_source_preflight()
    print("SpecRail gate wiring test passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
