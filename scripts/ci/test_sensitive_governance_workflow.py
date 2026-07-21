#!/usr/bin/env python3
"""Safety contract for the repo-local sensitive governance advisory."""

import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
WORKFLOW = ROOT / ".github" / "workflows" / "sensitive-governance.yml"
sys.path.insert(0, str(ROOT / "checks"))

from sensitive_enforcement import classify_sensitive_changes  # noqa: E402
from specrail_lib import load_pack  # noqa: E402


def test_trusted_base_classifier_uses_supported_source() -> None:
    result = classify_sensitive_changes(
        load_pack(ROOT),
        ROOT,
        ["workflow.yaml"],
        ["workflow.yaml"],
        source="github_changed_files",
    )
    assert result["source"] == "github_changed_files"
    assert result["enforcement_sensitive"] is True


def main() -> int:
    test_trusted_base_classifier_uses_supported_source()
    workflow = WORKFLOW.read_text(encoding="utf-8")

    required = [
        "pull_request_target:",
        "contents: read",
        "pull-requests: read",
        "ref: ${{ github.event.pull_request.base.sha }}",
        "persist-credentials: false",
        "pulls/$PR_NUMBER/files",
        "gh api --paginate --slurp",
        'sys.path.insert(0, "checks")',
        "from sensitive_enforcement import classify_sensitive_changes",
        'load_pack(Path("."))',
        'source="github_changed_files"',
        '"authorization": "advisory_only"',
        '"final_trust_root": "external_github_app_or_org_required_workflow_t6"',
        '"ordinary_pr_ci_is_final_authorization": False',
        "ordinary PR CI is not final governance authorization",
    ]
    for token in required:
        assert token in workflow, f"sensitive governance workflow is missing {token!r}"

    forbidden = [
        "contents: write",
        "issues: write",
        "pull-requests: write",
        "actions: write",
        "checks: write",
        "statuses: write",
        "github.event.pull_request.head.ref",
        "github.event.pull_request.head.sha",
        "github.event.pull_request.head.repo",
        "ref: main",
        "ref: ${{ github.event.repository.default_branch }}",
        "git fetch",
        "git checkout",
        "run_sensitive_implement_gate.py",
        "check_pr_tier.py",
    ]
    for token in forbidden:
        assert token not in workflow, f"sensitive governance workflow contains unsafe {token!r}"

    checkout = workflow.index("ref: ${{ github.event.pull_request.base.sha }}")
    api = workflow.index("pulls/$PR_NUMBER/files")
    classifier = workflow.index("from sensitive_enforcement import")
    assert checkout < api < classifier
    print("sensitive governance workflow safety tests passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
