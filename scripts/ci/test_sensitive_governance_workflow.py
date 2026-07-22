#!/usr/bin/env python3
"""Safety contract for the repo-local sensitive governance advisory."""

import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
WORKFLOW = ROOT / ".github" / "workflows" / "sensitive-governance.yml"
sys.path.insert(0, str(ROOT / "checks"))
sys.path.insert(0, str(ROOT / "scripts" / "ci"))

from sensitive_enforcement import classify_sensitive_changes  # noqa: E402
from specrail_lib import load_pack  # noqa: E402
from check_pr_tier import normalize_github_changed_file_pages  # noqa: E402


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


def test_changed_file_pages_are_complete_and_rename_aware() -> None:
    result = normalize_github_changed_file_pages(
        [[{"filename": "new", "previous_filename": "old", "status": "renamed"}]],
        1,
    )
    assert result["classification_paths"] == ["new", "old"]
    try:
        normalize_github_changed_file_pages(
            [[{"filename": "only", "status": "modified"}]], 2
        )
    except Exception as exc:
        assert "count does not match" in str(exc)
    else:
        raise AssertionError("partial changed-file pagination must fail closed")


def test_every_gate_schema_is_enforcement_sensitive() -> None:
    config = load_pack(ROOT)
    for schema in [
        "schemas/review_result.schema.json",
        "schemas/pr_review_gate.schema.json",
        "schemas/duplicate_work_evidence.schema.json",
        "schemas/runtime_checkpoint.schema.json",
    ]:
        result = classify_sensitive_changes(
            config, ROOT, [schema], [], source="github_changed_files"
        )
        assert result["enforcement_sensitive"] is True, schema


def main() -> int:
    test_trusted_base_classifier_uses_supported_source()
    test_changed_file_pages_are_complete_and_rename_aware()
    test_every_gate_schema_is_enforcement_sensitive()
    workflow = WORKFLOW.read_text(encoding="utf-8")

    required = [
        "pull_request_target:",
        "contents: read",
        "pull-requests: read",
        "Resolve fresh live repository and PR identity",
        'gh api "repos/$GITHUB_REPOSITORY/commits/$default_branch"',
        "ref: ${{ steps.live.outputs.base_sha }}",
        "persist-credentials: false",
        "Bind remote-tracking PR head without executing it",
        "scripts/ci/extract_nonclosing_issue.py",
        "scripts/ci/run_sensitive_implement_gate.py",
        'authorization: "advisory_only"',
        'final_trust_root: "external_github_app_or_org_required_workflow_t6"',
        "ordinary_pr_ci_is_final_authorization: false",
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
        "github.event.pull_request.base.sha",
        "github.event.pull_request.head.ref",
        "github.event.pull_request.head.sha",
        "github.event.pull_request.head.repo",
        "ref: main",
        "ref: ${{ github.event.repository.default_branch }}",
        "git checkout",
        "check_pr_tier.py",
    ]
    for token in forbidden:
        assert token not in workflow, f"sensitive governance workflow contains unsafe {token!r}"

    live = workflow.index("Resolve fresh live repository and PR identity")
    checkout = workflow.index("ref: ${{ steps.live.outputs.base_sha }}")
    bind = workflow.index("Bind remote-tracking PR head without executing it")
    gate = workflow.index("scripts/ci/run_sensitive_implement_gate.py")
    assert live < checkout < bind < gate
    print("sensitive governance workflow safety tests passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
