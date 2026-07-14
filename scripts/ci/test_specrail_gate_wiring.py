#!/usr/bin/env python3
"""Guard SpecRail runtime verification wiring in CI and local preflight."""

from __future__ import annotations

import importlib.util
import sys
from pathlib import Path
from types import ModuleType


ROOT = Path(__file__).resolve().parents[2]
CI_WORKFLOW = ROOT / ".github" / "workflows" / "ci.yml"
PREFLIGHT = ROOT / "scripts" / "ci" / "check_pr_preflight.py"
WIRING_TEST = ("python3", "scripts/ci/test_specrail_gate_wiring.py")
SYNC_VERIFY = ("scripts/sync-specrail-checks.sh", "--verify")


def load_preflight() -> ModuleType:
    spec = importlib.util.spec_from_file_location("_remem_pr_preflight", PREFLIGHT)
    if spec is None or spec.loader is None:
        raise AssertionError(f"cannot load {PREFLIGHT.relative_to(ROOT)}")
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


def main() -> int:
    assert_ordered(ci_run_commands(), "CI")

    module = load_preflight()
    steps = module.fast_steps("origin/main", "HEAD")
    preflight_commands = [tuple(command) for _name, command in steps]
    assert_ordered(preflight_commands, "fast/full preflight")

    print("SpecRail gate wiring test passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
