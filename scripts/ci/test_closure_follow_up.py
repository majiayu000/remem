#!/usr/bin/env python3
"""Focused tests for durable SpecRail closure follow-up persistence."""

from __future__ import annotations

import ast
import copy
import sys
import textwrap
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "checks"))
sys.path.insert(0, str(ROOT / "scripts" / "ci"))

from closure_audit import audit_closure  # noqa: E402
from closure_follow_up import FollowUpError, ensure_follow_up  # noqa: E402


HEAD = "a" * 40


class FakeGitHub:
    def __init__(self, issues: list[dict[str, Any]] | None = None) -> None:
        self.issues = copy.deepcopy(issues or [])
        self.created = 0
        self.reopened = 0
        self.fail_write = False
        self.corrupt_read_back: str | None = None

    def list_issues(self) -> list[dict[str, Any]]:
        return copy.deepcopy(self.issues)

    def create_issue(self, title: str, body: str) -> dict[str, Any]:
        if self.fail_write:
            raise FollowUpError("simulated create failure")
        self.created += 1
        issue = {
            "number": len(self.issues) + 1,
            "html_url": f"https://github.com/example/remem/issues/{len(self.issues) + 1}",
            "state": "open",
            "title": title,
            "body": body,
        }
        self.issues.append(issue)
        return copy.deepcopy(issue)

    def reopen_issue(self, number: int) -> dict[str, Any]:
        if self.fail_write:
            raise FollowUpError("simulated reopen failure")
        self.reopened += 1
        issue = self._find(number)
        issue["state"] = "open"
        return copy.deepcopy(issue)

    def get_issue(self, number: int) -> dict[str, Any]:
        issue = copy.deepcopy(self._find(number))
        if self.corrupt_read_back == "body":
            issue["body"] = "marker missing"
        elif self.corrupt_read_back == "title":
            issue["title"] = "wrong title"
        return issue

    def _find(self, number: int) -> dict[str, Any]:
        for issue in self.issues:
            if issue["number"] == number:
                return issue
        raise FollowUpError("simulated read-back failure")


def violation_audit() -> dict[str, Any]:
    return audit_closure(
        {
            "repository": "example/remem",
            "pr_number": 42,
            "final_head_sha": HEAD,
            "gate": None,
            "merge": {
                "merge_path": "merged_by_other",
                "remote_confirmed": True,
                "merged_at": "2026-07-21T00:01:00Z",
                "merged_head_sha": HEAD,
            },
        },
        checked_at="2026-07-21T00:02:00Z",
    )


def test_create_and_reuse() -> None:
    github = FakeGitHub()
    audit = violation_audit()
    first = ensure_follow_up(audit, repository="example/remem", github=github)
    second = ensure_follow_up(audit, repository="example/remem", github=github)
    assert first["status"] == "persisted"
    assert first["action"] == "created"
    assert second["action"] == "reused"
    assert first["issue"] == second["issue"]
    assert github.created == 1


def test_closed_issue_is_reopened() -> None:
    github = FakeGitHub()
    audit = violation_audit()
    created = ensure_follow_up(audit, repository="example/remem", github=github)
    github.issues[0]["state"] = "closed"
    reopened = ensure_follow_up(audit, repository="example/remem", github=github)
    assert reopened["action"] == "reopened"
    assert reopened["issue"]["number"] == created["issue"]["number"]
    assert github.reopened == 1


def test_compliant_audit_performs_no_write() -> None:
    audit = violation_audit()
    audit["status"] = "compliant"
    audit["violations"] = []
    audit["required_follow_up"] = None
    github = FakeGitHub()
    result = ensure_follow_up(audit, repository="example/remem", github=github)
    assert result["status"] == "not_required"
    assert github.created == 0


def test_api_write_failure_blocks_persistence() -> None:
    github = FakeGitHub()
    github.fail_write = True
    try:
        ensure_follow_up(violation_audit(), repository="example/remem", github=github)
    except FollowUpError as exc:
        assert "simulated create failure" in str(exc)
    else:
        raise AssertionError("GitHub write failure must block closure")


def test_read_back_mismatch_blocks_persistence() -> None:
    github = FakeGitHub()
    github.corrupt_read_back = "body"
    try:
        ensure_follow_up(violation_audit(), repository="example/remem", github=github)
    except FollowUpError as exc:
        assert "body does not match" in str(exc)
    else:
        raise AssertionError("unverified read-back must block closure")


def test_read_back_title_mismatch_blocks_persistence() -> None:
    github = FakeGitHub()
    github.corrupt_read_back = "title"
    try:
        ensure_follow_up(violation_audit(), repository="example/remem", github=github)
    except FollowUpError as exc:
        assert "title does not match" in str(exc)
    else:
        raise AssertionError("mismatched closure title must block persistence")


def test_preseeded_marker_with_wrong_fields_blocks_persistence() -> None:
    audit = violation_audit()
    follow_up = audit["required_follow_up"]
    marker = f"<!-- specrail-closure-follow-up:{follow_up['idempotency_key']} -->"
    github = FakeGitHub(
        [
            {
                "number": 7,
                "html_url": "https://github.com/example/remem/issues/7",
                "state": "open",
                "title": "unrelated issue",
                "body": marker,
            }
        ]
    )
    try:
        ensure_follow_up(audit, repository="example/remem", github=github)
    except FollowUpError as exc:
        assert "title does not match" in str(exc)
    else:
        raise AssertionError("preseeded marker must not bypass exact read-back")


def test_duplicate_markers_block_persistence() -> None:
    github = FakeGitHub()
    audit = violation_audit()
    ensure_follow_up(audit, repository="example/remem", github=github)
    duplicate = copy.deepcopy(github.issues[0])
    duplicate["number"] = 2
    duplicate["html_url"] = "https://github.com/example/remem/issues/2"
    github.issues.append(duplicate)
    try:
        ensure_follow_up(audit, repository="example/remem", github=github)
    except FollowUpError as exc:
        assert "multiple GitHub issues" in str(exc)
    else:
        raise AssertionError("duplicate durable records must fail closed")


def test_repository_mismatch_blocks_before_write() -> None:
    github = FakeGitHub()
    try:
        ensure_follow_up(violation_audit(), repository="other/remem", github=github)
    except FollowUpError as exc:
        assert "repository" in str(exc)
    else:
        raise AssertionError("repository mismatch must fail closed")
    assert github.created == 0


def test_workflow_uses_trusted_checkout_and_least_privilege() -> None:
    workflow = (ROOT / ".github" / "workflows" / "closure-audit.yml").read_text(
        encoding="utf-8"
    )
    required = [
        "pull_request_target:",
        "types: [closed]",
        "contents: read",
        "issues: write",
        "pull-requests: read",
        "github.event.pull_request.merged == true",
        "concurrency:",
        "closure-audit-pr-${{ github.event.pull_request.number }}",
        "Prove complete PR commits and trusted pre-merge base",
        "pulls/$PR_NUMBER/commits?per_page=100",
        "PR commit pagination is partial, duplicated, or count-drifted",
        "multi-commit rebase or ambiguous merge lacks a trusted pre-merge snapshot",
        '"pr_commits_complete": True',
        "ref: ${{ steps.trusted.outputs.base_sha }}",
        "persist-credentials: false",
        'jq \'{',
        '"$GITHUB_EVENT_PATH"',
        "checks/closure_audit.py",
        "scripts/ci/closure_follow_up.py",
        "closure-persistence-evidence.json",
        "persisted_follow_up",
        "Classify complete changed paths with trusted pre-merge registry",
        "classify_sensitive_changes",
        "normalize_github_changed_file_pages",
        'files["classification_paths"]',
        "gh api --paginate --slurp",
        "Repo-local compensation is not the T6 trust root",
        "final enforcement requires an external GitHub App",
    ]
    for token in required:
        assert token in workflow, f"closure workflow is missing {token!r}"
    forbidden = [
        "ref: main",
        "ref: ${{ github.event.repository.default_branch }}",
        "ref: ${{ github.event.pull_request.base.sha }}",
        "github.event.pull_request.head.ref",
        "github.event.pull_request.head.repo",
        "ref: ${{ github.event.pull_request.head.sha }}",
        "persist-credentials: true",
        "contains(github.event.pull_request.body",
    ]
    for token in forbidden:
        assert token not in workflow, f"closure workflow contains unsafe {token!r}"


def test_workflow_classification_cannot_be_shrunk_by_the_merged_pr() -> None:
    workflow = (ROOT / ".github" / "workflows" / "closure-audit.yml").read_text(
        encoding="utf-8"
    )
    commit_collection = workflow.index("pulls/$PR_NUMBER/commits?per_page=100")
    checkout = workflow.index("ref: ${{ steps.trusted.outputs.base_sha }}")
    changed_files = workflow.index("pulls/$PR_NUMBER/files?per_page=100")
    classification = workflow.index("from sensitive_enforcement import")
    controller = workflow.index("checks/closure_audit.py")

    assert commit_collection < checkout < changed_files < classification < controller
    assert 'sys.path.insert(0, "checks")' in workflow
    assert 'load_pack(Path("."))' in workflow
    assert "github.event.pull_request.head.sha }}" not in workflow
    assert "checkout" not in workflow[classification:controller].lower()
    assert 'status == "renamed"' not in workflow  # shared validator owns rename handling
    assert "previous_filename" not in workflow  # no ad-hoc filename-only collector


def test_trusted_base_selector_rejects_ambiguous_multi_commit_topology() -> None:
    workflow = (ROOT / ".github" / "workflows" / "closure-audit.yml").read_text(
        encoding="utf-8"
    )
    start = workflow.index("          def select_trusted_base")
    end = workflow.index("\n\n          commit_set =", start)
    tree = ast.parse(textwrap.dedent(workflow[start:end]))
    function = next(node for node in tree.body if isinstance(node, ast.FunctionDef))
    namespace: dict[str, Any] = {}
    code = compile(
        ast.Module(body=[function], type_ignores=[]),
        "<trusted-base-selector>",
        "exec",
    )
    exec(code, namespace)
    select = namespace["select_trusted_base"]

    base = "b" * 40
    commit = "c" * 40
    assert select([base], [commit], "d" * 40, 1) == (
        base,
        "single_commit_squash",
    )
    try:
        select([commit], ["1" * 40, "2" * 40], "3" * 40, 2)
    except SystemExit as exc:
        assert "trusted pre-merge snapshot" in str(exc)
    else:
        raise AssertionError("ambiguous multi-commit topology must fail closed")


def main() -> int:
    test_create_and_reuse()
    test_closed_issue_is_reopened()
    test_compliant_audit_performs_no_write()
    test_api_write_failure_blocks_persistence()
    test_read_back_mismatch_blocks_persistence()
    test_read_back_title_mismatch_blocks_persistence()
    test_preseeded_marker_with_wrong_fields_blocks_persistence()
    test_duplicate_markers_block_persistence()
    test_repository_mismatch_blocks_before_write()
    test_workflow_uses_trusted_checkout_and_least_privilege()
    test_workflow_classification_cannot_be_shrunk_by_the_merged_pr()
    test_trusted_base_selector_rejects_ambiguous_multi_commit_topology()
    print("closure follow-up controller tests passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
