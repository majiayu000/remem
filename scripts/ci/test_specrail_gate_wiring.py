#!/usr/bin/env python3
"""Guard SpecRail runtime verification wiring in CI and local preflight."""

from __future__ import annotations

import hashlib
import importlib.util
import json
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path
from types import ModuleType


ROOT = Path(__file__).resolve().parents[2]
CI_WORKFLOW = ROOT / ".github" / "workflows" / "ci.yml"
PREFLIGHT = ROOT / "scripts" / "ci" / "check_pr_preflight.py"
SYNC_SCRIPT = ROOT / "scripts" / "sync-specrail-checks.sh"
WORKFLOW_CHECK = ROOT / "checks" / "check_workflow.py"
WIRING_TEST = ("python3", "scripts/ci/test_specrail_gate_wiring.py")
SYNC_VERIFY = ("scripts/sync-specrail-checks.sh", "--verify")
TRANCHE_TEMPLATES = (
    "templates/tranche_checkpoint.md",
    "templates/zh-CN/tranche_checkpoint.md",
)
WORKFLOW_PACK_DIRS = (
    "checks",
    "policies",
    "review",
    "schemas",
    "skills",
    "templates",
    "tools",
)
WORKFLOW_PACK_FILES = (
    "AGENT_USAGE.md",
    "AGENTS.md",
    "labels.yaml",
    "skills-lock.json",
    "states.yaml",
    "workflow.yaml",
)


def load_preflight() -> ModuleType:
    spec = importlib.util.spec_from_file_location("_remem_pr_preflight", PREFLIGHT)
    if spec is None or spec.loader is None:
        raise AssertionError(f"cannot load {PREFLIGHT.relative_to(ROOT)}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def load_workflow_check() -> ModuleType:
    checks_dir = str(WORKFLOW_CHECK.parent)
    if checks_dir not in sys.path:
        sys.path.insert(0, checks_dir)
    spec = importlib.util.spec_from_file_location(
        "_remem_workflow_check", WORKFLOW_CHECK
    )
    if spec is None or spec.loader is None:
        raise AssertionError(f"cannot load {WORKFLOW_CHECK.relative_to(ROOT)}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def ci_run_commands() -> list[tuple[str, ...]]:
    commands: list[tuple[str, ...]] = []
    for line in CI_WORKFLOW.read_text(encoding="utf-8").splitlines():
        stripped = line.strip()
        if stripped.startswith("run:"):
            command = stripped.removeprefix("run:").strip()
            if command and command != "|":
                commands.append(tuple(command.split()))
    return commands


def assert_ordered(commands: list[tuple[str, ...]], label: str) -> None:
    assert WIRING_TEST in commands, f"{label} does not execute {WIRING_TEST}"
    assert SYNC_VERIFY in commands, (
        f"{label} does not execute runtime verifier {SYNC_VERIFY}; "
        "py_compile alone is insufficient"
    )
    assert commands.index(WIRING_TEST) < commands.index(SYNC_VERIFY), (
        f"{label} must test SpecRail wiring before running the verifier"
    )


def assert_managed_import_smoke() -> None:
    sync_text = SYNC_SCRIPT.read_text(encoding="utf-8")
    import_smoke_calls = sum(
        line.strip() == "verify_python_imports" for line in sync_text.splitlines()
    )
    assert import_smoke_calls == 2, (
        "sync and --verify must both execute the managed Python import smoke"
    )

    with tempfile.TemporaryDirectory(prefix="remem-specrail-import-smoke-") as raw:
        repo = Path(raw)
        target_script = repo / "scripts" / "sync-specrail-checks.sh"
        target_script.parent.mkdir(parents=True)
        shutil.copy2(SYNC_SCRIPT, target_script)

        lock_path = repo / "checks" / "specrail-sync.lock.json"
        lock_path.parent.mkdir(parents=True)
        lock = json.loads(
            (ROOT / "checks" / "specrail-sync.lock.json").read_text(encoding="utf-8")
        )
        for entry in lock["files"]:
            source = ROOT / entry["path"]
            target = repo / entry["path"]
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source, target)

        broken = repo / "checks" / "duplicate_work_gate.py"
        broken.write_text(
            broken.read_text(encoding="utf-8")
            + "\nimport specrail_missing_import_smoke\n",
            encoding="utf-8",
        )
        for entry in lock["files"]:
            if entry["path"] == "checks/duplicate_work_gate.py":
                entry["sha256"] = hashlib.sha256(broken.read_bytes()).hexdigest()
                break
        lock_path.write_text(json.dumps(lock, indent=2) + "\n", encoding="utf-8")

        completed = subprocess.run(
            [str(target_script), "--verify"],
            cwd=repo,
            capture_output=True,
            text=True,
            check=False,
        )
        assert completed.returncode != 0, "missing managed helper must fail --verify"
        assert "specrail_missing_import_smoke" in completed.stderr


def assert_missing_tranche_template_fails() -> None:
    lock = json.loads(
        (ROOT / "checks" / "specrail-sync.lock.json").read_text(encoding="utf-8")
    )
    managed_paths = {entry["path"] for entry in lock["files"]}
    assert set(TRANCHE_TEMPLATES) <= managed_paths, (
        "both tranche templates must be sync-managed"
    )

    with tempfile.TemporaryDirectory(prefix="remem-specrail-template-smoke-") as raw:
        repo = Path(raw)
        target_script = repo / "scripts" / "sync-specrail-checks.sh"
        target_script.parent.mkdir(parents=True)
        shutil.copy2(SYNC_SCRIPT, target_script)

        lock_path = repo / "checks" / "specrail-sync.lock.json"
        lock_path.parent.mkdir(parents=True)
        shutil.copy2(ROOT / "checks" / "specrail-sync.lock.json", lock_path)
        for entry in lock["files"]:
            source = ROOT / entry["path"]
            target = repo / entry["path"]
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source, target)

        missing = repo / TRANCHE_TEMPLATES[0]
        missing.unlink()
        completed = subprocess.run(
            [str(target_script), "--verify"],
            cwd=repo,
            capture_output=True,
            text=True,
            check=False,
        )
        assert completed.returncode != 0, "missing tranche template must fail --verify"
        assert f"MISSING: {TRANCHE_TEMPLATES[0]}" in completed.stdout


def assert_trusted_asset_validators() -> None:
    module = load_workflow_check()

    def run_with_helper(source: str) -> list[str]:
        with tempfile.TemporaryDirectory(prefix="remem-specrail-validator-smoke-") as raw:
            runner = Path(raw) / "checks" / "check_workflow.py"
            runner.parent.mkdir(parents=True)
            runner.write_text("# validation runner\n", encoding="utf-8")
            runner.with_name("pack_asset_validation.py").write_text(
                source,
                encoding="utf-8",
            )
            original_file = module.__file__
            module.__file__ = str(runner)
            try:
                return module.validate_pack_assets(ROOT)
            finally:
                module.__file__ = original_file

    assert run_with_helper("def validate_json_schemas(repo):\n    return []\n") == [
        "trusted pack asset validation missing callable validator(s): "
        "validate_template_parity"
    ]
    assert run_with_helper("def validate_template_parity(repo):\n    return []\n") == [
        "trusted pack asset validation missing callable validator(s): "
        "validate_json_schemas"
    ]
    assert run_with_helper(
        "def validate_json_schemas(repo):\n"
        "    return ['schema marker']\n"
        "def validate_template_parity(repo):\n"
        "    return ['template marker']\n"
    ) == ["schema marker", "template marker"]


def copy_workflow_pack(repo: Path) -> None:
    for relative_path in WORKFLOW_PACK_DIRS:
        shutil.copytree(
            ROOT / relative_path,
            repo / relative_path,
            ignore=shutil.ignore_patterns("__pycache__", "*.pyc"),
        )
    for relative_path in WORKFLOW_PACK_FILES:
        shutil.copy2(ROOT / relative_path, repo / relative_path)
    target_script = repo / "scripts" / "sync-specrail-checks.sh"
    target_script.parent.mkdir(parents=True)
    shutil.copy2(SYNC_SCRIPT, target_script)


def assert_invalid_sensitive_registries_fail() -> None:
    invalid_registries = (
        (
            "scalar paths",
            '    paths: "src/**"\n',
            "enforcement.sensitive_registry.paths must be a list",
        ),
        (
            "scalar specs",
            '    specs: "specs/**"\n',
            "enforcement.sensitive_registry.specs must be a list",
        ),
        (
            "unknown key",
            "    branches:\n      - main\n",
            "enforcement.sensitive_registry contains unsupported fields: branches",
        ),
        (
            "escaping path",
            '    paths:\n      - "../secrets/**"\n',
            "enforcement.sensitive_registry.paths[1] must stay within the repository",
        ),
        (
            "absolute path",
            '    paths:\n      - "/etc/**"\n',
            "enforcement.sensitive_registry.paths[1] must stay within the repository",
        ),
        (
            "Windows drive path",
            '    paths:\n      - "C:/secrets/**"\n',
            "enforcement.sensitive_registry.paths[1] must stay within the repository",
        ),
        (
            "empty pattern",
            '    paths:\n      - ""\n',
            "enforcement.sensitive_registry.paths[1] must be a non-empty string",
        ),
    )

    with tempfile.TemporaryDirectory(prefix="remem-specrail-registry-smoke-") as raw:
        repo = Path(raw)
        copy_workflow_pack(repo)
        baseline = (repo / "workflow.yaml").read_text(encoding="utf-8")
        for label, registry, expected_error in invalid_registries:
            (repo / "workflow.yaml").write_text(
                baseline
                + "\n"
                + "enforcement:\n"
                + "  sensitive_registry:\n"
                + registry,
                encoding="utf-8",
            )
            completed = subprocess.run(
                [
                    sys.executable,
                    str(repo / "checks" / "check_workflow.py"),
                    "--repo",
                    str(repo),
                ],
                cwd=repo,
                capture_output=True,
                text=True,
                check=False,
            )
            assert completed.returncode != 0, (
                f"invalid sensitive registry unexpectedly passed: {label}"
            )
            assert expected_error in completed.stdout, (
                f"invalid sensitive registry did not report {label}: "
                f"{completed.stdout}{completed.stderr}"
            )

        sync_completed = subprocess.run(
            [str(repo / "scripts" / "sync-specrail-checks.sh"), "--verify"],
            cwd=repo,
            capture_output=True,
            text=True,
            check=False,
        )
        assert sync_completed.returncode != 0, (
            "sync --verify must fail an invalid sensitive registry"
        )
        assert "SpecRail check passed" not in sync_completed.stdout
        assert invalid_registries[-1][2] in sync_completed.stdout


def main() -> int:
    assert_ordered(ci_run_commands(), "CI")

    module = load_preflight()
    steps = module.fast_steps("origin/main", "HEAD")
    preflight_commands = [tuple(command) for _name, command in steps]
    assert_ordered(preflight_commands, "fast/full preflight")
    assert_managed_import_smoke()
    assert_missing_tranche_template_fails()
    assert_trusted_asset_validators()
    assert_invalid_sensitive_registries_fail()

    print("SpecRail gate wiring test passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
