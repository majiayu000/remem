"""Collect maintainer-controlled approved-spec label evidence from GitHub."""

from __future__ import annotations

import re
from datetime import datetime
from typing import Any, Callable

from github_evidence_common import EvidenceError, json_array, json_object


APPROVAL_QUERY = """
query SpecRailApprovalLabels(
  $owner: String!, $name: String!, $number: Int!, $cursor: String
) {
  repository(owner: $owner, name: $name) {
    defaultBranchRef { name }
    issue(number: $number) {
      state
      labels(first: 100, after: $cursor) {
        pageInfo { hasNextPage endCursor }
        nodes { name }
      }
    }
  }
}
""".strip()

APPROVAL_TIMELINE_QUERY = """
query SpecRailApprovalTimeline(
  $owner: String!, $name: String!, $number: Int!, $cursor: String
) {
  repository(owner: $owner, name: $name) {
    defaultBranchRef { name }
    issue(number: $number) {
      state
      timelineItems(first: 100, after: $cursor, itemTypes: [LABELED_EVENT]) {
        pageInfo { hasNextPage endCursor }
        nodes {
          ... on LabeledEvent {
            createdAt
            actor { login }
            label { name }
          }
        }
      }
    }
  }
}
""".strip()


def collect_approval_metadata(
    github_repo: str,
    issue: int,
    run_json: Callable[[list[str]], Any],
    *,
    spec_source_commits: dict[str, str] | None = None,
) -> dict[str, Any]:
    owner, name = github_repo.split("/", 1)
    identity: tuple[str, str] | None = None

    def collect_connection(query: str, key: str) -> list[Any]:
        nonlocal identity
        cursor: str | None = None
        seen_cursors: set[str] = set()
        collected: list[Any] = []
        for _page in range(1000):
            args = [
                "api", "graphql", "-F", f"owner={owner}", "-F", f"name={name}",
                "-F", f"number={issue}", "-f", f"query={query}",
            ]
            if cursor is not None:
                args[2:2] = ["-F", f"cursor={cursor}"]
            payload = json_object(run_json(args), "approved-spec GraphQL response")
            try:
                repository = json_object(payload["data"]["repository"], "repository")
                issue_data = json_object(repository["issue"], "issue")
                default_branch = repository["defaultBranchRef"]["name"]
                connection = json_object(issue_data[key], key)
                nodes = json_array(connection["nodes"], f"{key}.nodes")
                page_info = json_object(connection["pageInfo"], f"{key}.pageInfo")
            except (KeyError, TypeError) as exc:
                raise EvidenceError("approved-spec query returned malformed issue evidence") from exc
            if not isinstance(default_branch, str) or not default_branch.strip():
                raise EvidenceError("approved-spec query lacks a trusted default branch")
            page_identity = (default_branch.strip(), str(issue_data.get("state")))
            if identity is None:
                identity = page_identity
            elif identity != page_identity:
                raise EvidenceError("approved-spec issue evidence drifted during pagination")
            collected.extend(nodes)
            has_next = page_info.get("hasNextPage")
            end_cursor = page_info.get("endCursor")
            if not isinstance(has_next, bool):
                raise EvidenceError(f"approved-spec {key} pageInfo is incomplete")
            if not has_next:
                return collected
            if not isinstance(end_cursor, str) or not end_cursor.strip() or end_cursor in seen_cursors:
                raise EvidenceError(f"approved-spec {key} pagination cursor is invalid")
            seen_cursors.add(end_cursor)
            cursor = end_cursor
        raise EvidenceError(f"approved-spec {key} pagination exceeded 1000 pages")

    labels = collect_connection(APPROVAL_QUERY, "labels")
    events = collect_connection(APPROVAL_TIMELINE_QUERY, "timelineItems")
    assert identity is not None
    default_branch, issue_state = identity
    if issue_state != "OPEN":
        raise EvidenceError("approved-spec issue must remain OPEN")
    current_labels = {
        item.get("name") for item in labels
        if isinstance(item, dict) and isinstance(item.get("name"), str)
    }
    if "ready_to_implement" not in current_labels:
        raise EvidenceError(
            "approved spec requires current ready_to_implement maintainer label"
        )
    candidates: list[tuple[str, str]] = []
    for event in events:
        if not isinstance(event, dict):
            continue
        label = event.get("label")
        actor = event.get("actor")
        if not isinstance(label, dict) or label.get("name") != "ready_to_implement":
            continue
        created_at = event.get("createdAt")
        login = actor.get("login") if isinstance(actor, dict) else None
        if isinstance(created_at, str) and created_at.strip() and isinstance(login, str) and login.strip():
            candidates.append((created_at.strip(), login.strip()))
    if not candidates:
        raise EvidenceError(
            "ready_to_implement label lacks maintainer actor/timestamp evidence"
        )
    approved_at, maintainer_actor = max(candidates)
    result: dict[str, Any] = {
        "approved_at": approved_at,
        "maintainer_actor": maintainer_actor,
        "state_source": "label",
        "state_trusted": True,
    }
    if spec_source_commits is None:
        return result
    try:
        approved_time = datetime.fromisoformat(approved_at.replace("Z", "+00:00"))
    except ValueError as exc:
        raise EvidenceError("ready_to_implement label timestamp is invalid") from exc
    if approved_time.tzinfo is None:
        raise EvidenceError("ready_to_implement label timestamp must include timezone")
    revisions: dict[str, Any] = {}
    for path, source_commit in spec_source_commits.items():
        if not re.fullmatch(r"[0-9a-fA-F]{40}", source_commit):
            raise EvidenceError(f"approved spec source commit is invalid: {path}")
        pulls = json_array(run_json(
            [
                "api", "--method", "GET",
                f"repos/{owner}/{name}/commits/{source_commit}/pulls",
            ]
        ), f"associated PR response for {path}")
        candidates: list[dict[str, Any]] = []
        for pull in pulls:
            if not isinstance(pull, dict):
                continue
            base = pull.get("base")
            merged_at = pull.get("merged_at")
            merge_commit = pull.get("merge_commit_sha")
            number = pull.get("number")
            if not isinstance(base, dict) or base.get("ref") != default_branch:
                continue
            if not isinstance(merged_at, str) or not isinstance(merge_commit, str):
                continue
            if not isinstance(number, int) or isinstance(number, bool) or number <= 0:
                continue
            try:
                merged_time = datetime.fromisoformat(merged_at.replace("Z", "+00:00"))
            except ValueError:
                continue
            if merged_time.tzinfo is None or merged_time > approved_time:
                continue
            if not re.fullmatch(r"[0-9a-fA-F]{40}", merge_commit):
                continue
            candidates.append(pull)
        if len(candidates) != 1:
            raise EvidenceError(
                f"approved spec source must have exactly one merged default-branch PR: {path}"
            )
        pull = candidates[0]
        revisions[path] = {
            "source_commit_sha": source_commit.lower(),
            "pr_number": pull["number"],
            "merged_at": pull["merged_at"],
            "merge_commit_sha": pull["merge_commit_sha"].lower(),
        }
    result["spec_revisions"] = revisions
    return result
