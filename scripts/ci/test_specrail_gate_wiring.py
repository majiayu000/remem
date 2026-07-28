#!/usr/bin/env python3
"""Regression contract keeping mechanical SpecRail gates disabled."""

from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
WORKFLOW_DIR = ROOT / ".github" / "workflows"
CI_WORKFLOW = WORKFLOW_DIR / "ci.yml"
PREFLIGHT = ROOT / "scripts" / "ci" / "check_pr_preflight.py"

REMOVED_WORKFLOWS = (
    WORKFLOW_DIR / "sensitive-governance.yml",
    WORKFLOW_DIR / "closure-audit.yml",
)

MECHANICAL_GATE_TOKENS = (
    "checks/pr_gate.py",
    "checks/closure_audit.py",
    "scripts/ci/check_pr_tier.py",
    "scripts/ci/run_sensitive_implement_gate.py",
    "scripts/ci/extract_nonclosing_issue.py",
    "scripts/ci/closure_follow_up.py",
    "scripts/ci/test_sensitive_governance_workflow.py",
    "scripts/ci/test_closure_follow_up.py",
    "scripts/ci/test_run_sensitive_implement_gate.py",
    "scripts/ci/test_extract_nonclosing_issue.py",
    "scripts/sync-specrail-checks.sh",
    ".specrail/runtime",
    "enforcement_sensitive",
    "ready_to_implement",
    "refresh_after_readiness_change",
    "Pin trusted default branch",
)

RETAINED_CI_TOKENS = (
    "python3 scripts/ci/test_specrail_gate_wiring.py",
    "python3 scripts/ci/check_plugin_version_sync.py",
    "python3 scripts/ci/check_public_surface.py",
    "python3 scripts/ci/check_public_claims.py",
    "python3 scripts/ci/check_file_size.py",
    "python3 scripts/ci/check_release_workflows.py",
    "node --test",
    "python3 scripts/ci/check_spec_lifecycle.py",
    "python3 scripts/ci/check_version_bump.py",
    "cargo fmt --check",
    "cargo clippy --all-targets -- -D warnings",
    "scripts/smoke_native_web_api.sh",
    "cargo run -- eval-extraction --json --check-baseline",
    "cargo run -- eval-gates",
    "cargo test",
)


def test_removed_gate_workflows_stay_absent() -> None:
    for workflow in REMOVED_WORKFLOWS:
        assert not workflow.exists(), f"mechanical gate workflow restored: {workflow}"


def test_no_workflow_or_preflight_rewires_mechanical_gates() -> None:
    inspected = [*WORKFLOW_DIR.glob("*.yml"), *WORKFLOW_DIR.glob("*.yaml"), PREFLIGHT]
    for path in inspected:
        text = path.read_text(encoding="utf-8")
        for token in MECHANICAL_GATE_TOKENS:
            assert token not in text, f"{path} rewires mechanical gate token {token!r}"


def test_normal_quality_ci_stays_enabled() -> None:
    ci = CI_WORKFLOW.read_text(encoding="utf-8")
    for token in RETAINED_CI_TOKENS:
        assert token in ci, f"normal CI check was removed: {token!r}"


def test_offline_specrail_pack_is_retained() -> None:
    retained = (
        ROOT / "checks" / "pr_gate.py",
        ROOT / "checks" / "check_workflow.py",
        ROOT / "scripts" / "sync-specrail-checks.sh",
    )
    for path in retained:
        assert path.is_file(), f"offline SpecRail reference was removed: {path}"


def main() -> int:
    test_removed_gate_workflows_stay_absent()
    test_no_workflow_or_preflight_rewires_mechanical_gates()
    test_normal_quality_ci_stays_enabled()
    test_offline_specrail_pack_is_retained()
    print("mechanical SpecRail gates are disabled; normal CI remains enabled")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
