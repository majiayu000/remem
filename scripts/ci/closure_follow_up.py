#!/usr/bin/env python3
"""Persist SpecRail closure violations as durable GitHub issues."""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
from pathlib import Path
from typing import Any, Protocol


ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "checks"))

from specrail_lib import SpecRailError, resolve_repo_path, validate_instance  # noqa: E402


class FollowUpError(RuntimeError):
    """Raised when a required follow-up cannot be durably verified."""


class GitHubIssues(Protocol):
    def list_issues(self) -> list[dict[str, Any]]: ...

    def create_issue(self, title: str, body: str) -> dict[str, Any]: ...

    def reopen_issue(self, number: int) -> dict[str, Any]: ...

    def get_issue(self, number: int) -> dict[str, Any]: ...


class GhIssues:
    """GitHub Issues adapter using argument arrays rather than shell commands."""

    def __init__(self, repository: str) -> None:
        self.repository = repository

    def _run(self, args: list[str]) -> Any:
        try:
            completed = subprocess.run(
                ["gh", "api", *args],
                check=False,
                capture_output=True,
                text=True,
            )
        except OSError as exc:
            raise FollowUpError(f"cannot execute GitHub API client: {exc}") from exc
        if completed.returncode != 0:
            detail = completed.stderr.strip() or completed.stdout.strip()
            raise FollowUpError(f"GitHub API request failed: {detail or 'unknown error'}")
        try:
            return json.loads(completed.stdout)
        except json.JSONDecodeError as exc:
            raise FollowUpError("GitHub API returned invalid JSON") from exc

    def list_issues(self) -> list[dict[str, Any]]:
        pages = self._run(
            [
                "--method",
                "GET",
                "--paginate",
                "--slurp",
                f"repos/{self.repository}/issues",
                "-f",
                "state=all",
                "-f",
                "per_page=100",
            ]
        )
        if not isinstance(pages, list) or any(not isinstance(page, list) for page in pages):
            raise FollowUpError("GitHub issue listing returned an unexpected shape")
        return [item for page in pages for item in page if isinstance(item, dict)]

    def create_issue(self, title: str, body: str) -> dict[str, Any]:
        result = self._run(
            [
                "--method",
                "POST",
                f"repos/{self.repository}/issues",
                "-f",
                f"title={title}",
                "-f",
                f"body={body}",
            ]
        )
        if not isinstance(result, dict):
            raise FollowUpError("GitHub issue creation returned an unexpected shape")
        return result

    def reopen_issue(self, number: int) -> dict[str, Any]:
        result = self._run(
            [
                "--method",
                "PATCH",
                f"repos/{self.repository}/issues/{number}",
                "-f",
                "state=open",
            ]
        )
        if not isinstance(result, dict):
            raise FollowUpError("GitHub issue reopen returned an unexpected shape")
        return result

    def get_issue(self, number: int) -> dict[str, Any]:
        result = self._run([f"repos/{self.repository}/issues/{number}"])
        if not isinstance(result, dict):
            raise FollowUpError("GitHub issue read-back returned an unexpected shape")
        return result


def _load_audit(repo: Path, raw_path: str) -> dict[str, Any]:
    audit_path = resolve_repo_path(repo, raw_path, label="closure audit result")
    try:
        value = json.loads(audit_path.read_text(encoding="utf-8"))
    except OSError as exc:
        raise FollowUpError(f"cannot read closure audit result: {exc}") from exc
    except json.JSONDecodeError as exc:
        raise FollowUpError(f"invalid closure audit result JSON: {exc.msg}") from exc
    if not isinstance(value, dict):
        raise FollowUpError("closure audit result must be an object")
    schema_path = resolve_repo_path(
        repo,
        "schemas/closure_audit_result.schema.json",
        label="closure audit schema",
    )
    try:
        schema = json.loads(schema_path.read_text(encoding="utf-8"))
        validate_instance(schema, value, "closure audit result")
    except (OSError, json.JSONDecodeError, SpecRailError) as exc:
        raise FollowUpError(f"closure audit result failed schema validation: {exc}") from exc
    return value


def _marker(idempotency_key: str) -> str:
    return f"<!-- specrail-closure-follow-up:{idempotency_key} -->"


def _issue_body(follow_up: dict[str, Any]) -> str:
    repository = follow_up["repository"]
    pr_number = follow_up["pr_number"]
    return "\n".join(
        [
            _marker(follow_up["idempotency_key"]),
            "## Closure audit violation",
            "",
            follow_up["summary"],
            "",
            f"- Pull request: https://github.com/{repository}/pull/{pr_number}",
            f"- Final head: `{follow_up['final_head_sha']}`",
            f"- Violation: `{follow_up['violation_code']}`",
            f"- Idempotency key: `{follow_up['idempotency_key']}`",
            "",
            "This issue is a durable follow-up record. Closing it requires fresh exact-head",
            "review, CI, thread, and closure evidence; the audit artifact alone is insufficient.",
        ]
    )


def _validate_follow_up(value: Any, repository: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise FollowUpError("violation audit is missing required_follow_up")
    required = {
        "violation_code",
        "repository",
        "pr_number",
        "final_head_sha",
        "idempotency_key",
        "summary",
    }
    if set(value) != required:
        raise FollowUpError("required_follow_up fields do not match the closure contract")
    if value.get("repository") != repository:
        raise FollowUpError("required_follow_up repository does not match --github-repo")
    return value


def _issue_number(issue: dict[str, Any]) -> int:
    number = issue.get("number")
    if not isinstance(number, int) or isinstance(number, bool) or number <= 0:
        raise FollowUpError("GitHub issue is missing a valid number")
    return number


def _verify_read_back(issue: dict[str, Any], marker: str) -> dict[str, Any]:
    number = _issue_number(issue)
    url = issue.get("html_url")
    state = issue.get("state")
    body = issue.get("body")
    if not isinstance(url, str) or not url.strip():
        raise FollowUpError("GitHub issue read-back is missing html_url")
    if state != "open":
        raise FollowUpError("GitHub issue read-back is not open")
    if not isinstance(body, str) or marker not in body:
        raise FollowUpError("GitHub issue read-back does not contain the idempotency marker")
    return {"number": number, "url": url, "state": state}


def ensure_follow_up(
    audit: dict[str, Any],
    *,
    repository: str,
    github: GitHubIssues,
) -> dict[str, Any]:
    if audit.get("repository") != repository:
        raise FollowUpError("closure audit repository does not match --github-repo")
    if audit.get("status") == "compliant":
        return {
            "version": 1,
            "status": "not_required",
            "repository": repository,
            "persisted": False,
            "issue": None,
        }
    if audit.get("status") != "violation":
        raise FollowUpError("closure audit status must be compliant or violation")

    follow_up = _validate_follow_up(audit.get("required_follow_up"), repository)
    marker = _marker(follow_up["idempotency_key"])
    matches = [
        issue
        for issue in github.list_issues()
        if "pull_request" not in issue
        and isinstance(issue.get("body"), str)
        and marker in issue["body"]
    ]
    if len(matches) > 1:
        raise FollowUpError("multiple GitHub issues contain the same idempotency marker")

    action = "reused"
    if matches:
        number = _issue_number(matches[0])
        if matches[0].get("state") != "open":
            github.reopen_issue(number)
            action = "reopened"
    else:
        created = github.create_issue(
            f"[SpecRail closure] PR #{follow_up['pr_number']} {follow_up['violation_code']}",
            _issue_body(follow_up),
        )
        number = _issue_number(created)
        action = "created"

    verified = _verify_read_back(github.get_issue(number), marker)
    return {
        "version": 1,
        "status": "persisted",
        "repository": repository,
        "persisted": True,
        "idempotency_key": follow_up["idempotency_key"],
        "action": action,
        "issue": verified,
    }


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description="Persist a SpecRail closure violation as an idempotent GitHub issue."
    )
    parser.add_argument("--repo", default=".", help="Repository root")
    parser.add_argument("--audit-result", required=True, help="Closure audit result JSON")
    parser.add_argument("--github-repo", required=True, help="GitHub repository as OWNER/REPO")
    parser.add_argument("--json", action="store_true", help="Print JSON output")
    args = parser.parse_args(argv)
    try:
        repo = Path(args.repo).resolve()
        audit = _load_audit(repo, args.audit_result)
        result = ensure_follow_up(
            audit,
            repository=args.github_repo.lower(),
            github=GhIssues(args.github_repo.lower()),
        )
    except (FollowUpError, SpecRailError) as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 2
    print(json.dumps(result, ensure_ascii=False, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
