#!/usr/bin/env python3
"""Run the prospective enforcement-sensitive implementation gate.

This repository-local wrapper deliberately owns the live GitHub bindings that the
synced SpecRail collectors do not provide.  It fails closed on every unavailable,
ambiguous, stale, or mutable input.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import subprocess
import sys
import tempfile
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Callable


WRAPPER_ID = "remem-sensitive-implement-gate/v1"
READY_LABEL = "ready_to_implement"
MAX_EVIDENCE_AGE_SECONDS = 300
FULL_SHA = re.compile(r"^[0-9a-fA-F]{40}$")
REPO_NAME = re.compile(r"^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$")
COMMENT_URL = re.compile(
    r"https://github\.com/(?P<repo>[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+)/"
    r"issues/(?P<issue>[1-9][0-9]*)#issuecomment-(?P<comment>[1-9][0-9]*)"
)
FIELD_LINE = re.compile(r"^\s*([A-Za-z][A-Za-z -]*):\s*(.*?)\s*$")


class GateError(RuntimeError):
    """An evidence failure that must block implementation."""


Runner = Callable[[list[str]], subprocess.CompletedProcess[str]]


@dataclass(frozen=True)
class Config:
    repo: Path
    github_repo: str
    issue: int
    pr: int
    head_sha: str
    output: Path
    pr_limit: int = 100


def utc_now() -> datetime:
    return datetime.now(timezone.utc)


def iso8601(value: datetime) -> str:
    return value.astimezone(timezone.utc).replace(microsecond=0).isoformat().replace(
        "+00:00", "Z"
    )


def default_runner(argv: list[str]) -> subprocess.CompletedProcess[str]:
    try:
        return subprocess.run(argv, check=False, capture_output=True, text=True)
    except FileNotFoundError as exc:
        raise GateError(f"required executable is unavailable: {argv[0]}") from exc


def run(runner: Runner, argv: list[str]) -> str:
    completed = runner(argv)
    if completed.returncode != 0:
        detail = completed.stderr.strip() or completed.stdout.strip() or "no output"
        raise GateError(f"command failed ({argv[0]}): {detail}")
    return completed.stdout


def run_json(runner: Runner, argv: list[str], label: str) -> Any:
    raw = run(runner, argv)
    try:
        return json.loads(raw)
    except json.JSONDecodeError as exc:
        raise GateError(f"{label} returned invalid JSON: {exc.msg}") from exc


def object_value(value: Any, label: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise GateError(f"{label} must be a JSON object")
    return value


def canonical_bytes(value: Any) -> bytes:
    return (json.dumps(value, sort_keys=True, separators=(",", ":")) + "\n").encode()


def value_hash(value: Any) -> str:
    return hashlib.sha256(canonical_bytes(value)).hexdigest()


def file_hash(path: Path) -> str:
    try:
        return hashlib.sha256(path.read_bytes()).hexdigest()
    except OSError as exc:
        raise GateError(f"cannot hash evidence file {path}: {exc}") from exc


def parse_time(raw: Any, label: str) -> datetime:
    if not isinstance(raw, str) or not raw.strip():
        raise GateError(f"{label} must be a non-empty timestamp")
    try:
        parsed = datetime.fromisoformat(raw.replace("Z", "+00:00"))
    except ValueError as exc:
        raise GateError(f"{label} is not a valid ISO-8601 timestamp") from exc
    if parsed.tzinfo is None:
        raise GateError(f"{label} must include a timezone")
    return parsed.astimezone(timezone.utc)


def normalize_remote(raw: str) -> str:
    value = raw.strip().removesuffix(".git").rstrip("/")
    ssh = re.fullmatch(r"git@github\.com:([^/]+/[^/]+)", value)
    if ssh:
        return ssh.group(1)
    https = re.fullmatch(r"https://github\.com/([^/]+/[^/]+)", value)
    if https:
        return https.group(1)
    raise GateError("local origin must be a github.com SSH or HTTPS repository URL")


def text_field(payload: dict[str, Any], name: str, label: str) -> str:
    value = payload.get(name)
    if not isinstance(value, str) or not value.strip():
        raise GateError(f"{label}.{name} must be a non-empty string")
    return value.strip()


def positive_int(payload: dict[str, Any], name: str, label: str) -> int:
    value = payload.get(name)
    if not isinstance(value, int) or isinstance(value, bool) or value <= 0:
        raise GateError(f"{label}.{name} must be a positive integer")
    return value


def references_issue(text: str, issue: int) -> bool:
    patterns = [
        rf"(?<![A-Za-z0-9])(?:GH-?{issue}|#{issue})(?![A-Za-z0-9])",
        rf"[\w.-]+/[\w.-]+#{issue}(?![0-9])",
        rf"https?://\S+/issues/{issue}(?![0-9])",
    ]
    return any(re.search(pattern, text, re.IGNORECASE) for pattern in patterns)


def validate_actor(login: str, actor_type: str) -> None:
    lowered = login.lower()
    if actor_type != "User" or "[bot]" in lowered or lowered.endswith("bot") or "agent" in lowered:
        raise GateError("readiness/decision actor must be a non-bot, non-app, non-agent User")


def authority_for(
    runner: Runner, github_repo: str, login: str, actor_type: str
) -> dict[str, Any]:
    validate_actor(login, actor_type)
    owner = github_repo.split("/", 1)[0]
    encoded_login = login.replace("/", "%2F")
    payload = object_value(
        run_json(
            runner,
            ["gh", "api", f"repos/{github_repo}/collaborators/{encoded_login}/permission"],
            "repository permission query",
        ),
        "repository permission query",
    )
    permission = payload.get("permission")
    user = payload.get("user")
    if isinstance(user, dict) and isinstance(user.get("type"), str):
        validate_actor(login, user["type"])
    if permission not in {"admin", "maintain"}:
        raise GateError("readiness/decision actor lacks live admin or maintain permission")
    return {
        "actor": login,
        "actor_type": actor_type,
        "permission": permission,
        "role": "owner" if login.casefold() == owner.casefold() else "maintainer",
        "authority_source": f"gh:repos/{github_repo}/collaborators/{login}/permission",
    }


def latest_ready_event(
    runner: Runner, github_repo: str, issue: int
) -> tuple[dict[str, Any], dict[str, Any]]:
    payload = run_json(
        runner,
        ["gh", "api", "--paginate", "--slurp", f"repos/{github_repo}/issues/{issue}/events"],
        "issue label event query",
    )
    pages = payload if isinstance(payload, list) else None
    if pages is None:
        raise GateError("issue label event query must return a page list")
    events: list[dict[str, Any]] = []
    for page in pages:
        if not isinstance(page, list):
            raise GateError("issue label event query contains a non-list page")
        for item in page:
            if not isinstance(item, dict):
                raise GateError("issue label event must be an object")
            label = item.get("label")
            if item.get("event") == "labeled" and isinstance(label, dict) and label.get("name") == READY_LABEL:
                events.append(item)
    if not events:
        raise GateError(f"no {READY_LABEL} label event exists")
    ranked = sorted(events, key=lambda item: parse_time(item.get("created_at"), "label event created_at"))
    event = ranked[-1]
    actor = object_value(event.get("actor"), "label event actor")
    login = text_field(actor, "login", "label event actor")
    actor_type = text_field(actor, "type", "label event actor")
    authority = authority_for(runner, github_repo, login, actor_type)
    return {
        "label": READY_LABEL,
        "event": "labeled",
        "actor": login,
        "actor_type": actor_type,
        "created_at": iso8601(parse_time(event.get("created_at"), "label event created_at")),
        "event_id": event.get("id"),
        "source": f"gh:repos/{github_repo}/issues/{issue}/events",
    }, authority


def validate_duplicate(value: Any, issue: int) -> dict[str, Any]:
    payload = object_value(value, "duplicate evidence")
    required = {"issue", "collected_at", "open_prs_complete", "open_pr_limit", "open_prs", "remote_branches"}
    if set(payload) != required:
        raise GateError("duplicate evidence fields do not match the synced schema")
    if payload.get("issue") != issue or payload.get("open_prs_complete") is not True:
        raise GateError("duplicate evidence issue mismatch or incomplete open PR list")
    if not isinstance(payload.get("open_pr_limit"), int) or payload["open_pr_limit"] <= 0:
        raise GateError("duplicate evidence open_pr_limit is invalid")
    if not isinstance(payload.get("open_prs"), list) or not isinstance(payload.get("remote_branches"), list):
        raise GateError("duplicate evidence PRs and branches must be arrays")
    for item in payload["open_prs"]:
        if not isinstance(item, dict) or set(item) != {"number", "head_ref", "references_issue"}:
            raise GateError("duplicate evidence PR item is schema-invalid")
        positive_int(item, "number", "duplicate PR")
        text_field(item, "head_ref", "duplicate PR")
        if not isinstance(item.get("references_issue"), bool):
            raise GateError("duplicate PR references_issue must be boolean")
    if not all(isinstance(branch, str) and branch.strip() for branch in payload["remote_branches"]):
        raise GateError("duplicate remote branch names must be non-empty strings")
    return payload


def check_freshness(payload: dict[str, Any], started: datetime) -> float:
    collected = parse_time(payload.get("collected_at"), "duplicate evidence collected_at")
    age = (started - collected).total_seconds()
    if age < 0 or age > MAX_EVIDENCE_AGE_SECONDS:
        raise GateError("duplicate evidence is future-dated or older than 300 seconds")
    return age


def matching_branches(repo: Path, issue: int, branches: list[str]) -> list[str]:
    sys.path.insert(0, str(repo / "checks"))
    try:
        from duplicate_work_gate import impl_branch_token, matching_contract_branches
        from specrail_lib import load_pack
    except ImportError as exc:
        raise GateError(f"cannot load synced duplicate gate: {exc}") from exc
    token = impl_branch_token(load_pack(repo), issue)
    if token is None:
        raise GateError("workflow implementation branch token is unavailable")
    return matching_contract_branches(branches, token)


def parse_decision_body(body: str) -> dict[str, Any]:
    fields: dict[str, str] = {}
    branches: list[str] = []
    in_branches = False
    for line in body.splitlines():
        match = FIELD_LINE.match(line)
        if match:
            key = match.group(1).lower().replace(" ", "-")
            value = match.group(2).strip().strip("`")
            if key == "branches":
                in_branches = True
                if value:
                    branches.extend(part.strip() for part in value.split(",") if part.strip())
            else:
                in_branches = False
                fields[key] = value
            continue
        if in_branches and re.match(r"^\s*[-*]\s+", line):
            branches.append(re.sub(r"^\s*[-*]\s+", "", line).strip().strip("`"))
    required = {"repository", "issue", "pr", "head-sha", "decision", "rationale"}
    if not required.issubset(fields) or not branches:
        raise GateError("branch decision comment lacks binding fields, rationale, or branches")
    return {**fields, "branches": sorted(set(branches))}


def ownership_decision(
    runner: Runner,
    config: Config,
    pr_body: str,
    conflicting_branches: list[str],
) -> tuple[dict[str, Any] | None, set[str]]:
    if not conflicting_branches:
        return None, set()
    urls = [
        match
        for match in COMMENT_URL.finditer(pr_body)
        if match.group("repo").casefold() == config.github_repo.casefold()
        and int(match.group("issue")) == config.issue
    ]
    candidates: list[tuple[re.Match[str], dict[str, Any], str]] = []
    for match in urls:
        comment_id = int(match.group("comment"))
        comment = object_value(
            run_json(
                runner,
                [
                    "gh",
                    "api",
                    f"repos/{config.github_repo}/issues/comments/{comment_id}",
                ],
                "branch decision comment",
            ),
            "branch decision comment",
        )
        body = text_field(comment, "body", "branch decision comment")
        if "SpecRail branch ownership decision" in body:
            candidates.append((match, comment, body))
    if len(candidates) != 1:
        raise GateError(
            "conflicting branches require exactly one explicit SpecRail branch "
            "ownership decision comment in the PR body"
        )
    match, comment, body = candidates[0]
    comment_id = int(match.group("comment"))
    decision = parse_decision_body(body)
    expected = {
        "repository": config.github_repo,
        "issue": str(config.issue),
        "pr": str(config.pr),
        "head-sha": config.head_sha,
    }
    for field, value in expected.items():
        if decision[field].casefold() != value.casefold():
            raise GateError(f"branch decision {field} does not bind the current implementation")
    if decision["decision"] not in {"continue_existing_work", "cleanup_completed"}:
        raise GateError("branch decision is not an allowed ownership decision")
    listed = set(decision["branches"])
    unknown = listed.difference(conflicting_branches)
    if unknown:
        raise GateError(f"branch decision lists non-conflicting branches: {sorted(unknown)}")
    remaining = set(conflicting_branches).difference(listed)
    if remaining:
        raise GateError(f"branch decision does not cover all conflicting branches: {sorted(remaining)}")
    user = object_value(comment.get("user"), "branch decision user")
    login = text_field(user, "login", "branch decision user")
    actor_type = text_field(user, "type", "branch decision user")
    authority = authority_for(runner, config.github_repo, login, actor_type)
    created_at = iso8601(parse_time(comment.get("created_at"), "branch decision created_at"))
    html_url = text_field(comment, "html_url", "branch decision comment")
    expected_url = (
        f"https://github.com/{config.github_repo}/issues/{config.issue}"
        f"#issuecomment-{comment_id}"
    )
    if html_url != expected_url:
        raise GateError("branch decision comment URL mismatch")
    return {
        "url": html_url,
        "comment_id": comment_id,
        "actor": login,
        "actor_type": actor_type,
        "created_at": created_at,
        "rationale": decision["rationale"],
        "decision": decision["decision"],
        "branches": sorted(listed),
        "authority": authority,
    }, listed


def filter_duplicates(
    payload: dict[str, Any], config: Config, head_ref: str, decided_branches: set[str]
) -> tuple[dict[str, Any], dict[str, Any]]:
    filtered = json.loads(json.dumps(payload))
    pr_exemptions: list[dict[str, Any]] = []
    kept_prs = []
    for item in filtered["open_prs"]:
        if item["references_issue"] and item["number"] == config.pr and item["head_ref"] == head_ref:
            pr_exemptions.append({"number": config.pr, "head_ref": head_ref, "head_sha": config.head_sha})
        else:
            kept_prs.append(item)
    if len(pr_exemptions) != 1:
        raise GateError("duplicate evidence must contain exactly one current PR/head-ref self-reference")
    branch_exemptions: list[dict[str, Any]] = []
    kept_branches = []
    for branch in filtered["remote_branches"]:
        if branch == head_ref:
            branch_exemptions.append({"head_ref": head_ref, "head_sha": config.head_sha, "source": "live_pr_payload"})
        elif branch in decided_branches:
            branch_exemptions.append({"head_ref": branch, "source": "human_branch_ownership_decision"})
        else:
            kept_branches.append(branch)
    if len([item for item in branch_exemptions if item["source"] == "live_pr_payload"]) != 1:
        raise GateError("duplicate evidence must contain the unique live PR remote head branch")
    filtered["open_prs"] = kept_prs
    filtered["remote_branches"] = kept_branches
    return filtered, {"prs": pr_exemptions, "branches": branch_exemptions}


def validate_route(result: Any) -> dict[str, Any]:
    route = object_value(result, "route gate result")
    classification = route.get("sensitive_classification")
    if route.get("decision") != "allowed" or route.get("missing") != []:
        raise GateError("nested route gate is not allowed with missing=[]")
    if route.get("current_state") != READY_LABEL:
        raise GateError("nested route gate current_state is not ready_to_implement")
    if not isinstance(route.get("satisfied"), list) or not any(
        isinstance(item, str) and "state provided by evidence" in item for item in route["satisfied"]
    ):
        raise GateError("nested route gate does not attest evidence-derived state")
    if not isinstance(classification, dict) or classification.get("enforcement_sensitive") is not True:
        raise GateError("nested route gate did not classify the change as enforcement-sensitive")
    return route


def execute(config: Config, runner: Runner = default_runner, now: Callable[[], datetime] = utc_now) -> dict[str, Any]:
    started = now().astimezone(timezone.utc)
    if not REPO_NAME.fullmatch(config.github_repo):
        raise GateError("github repository must use OWNER/REPO format")
    if config.issue <= 0 or config.pr <= 0 or config.pr_limit <= 0:
        raise GateError("issue, PR, and PR limit must be positive")
    if not FULL_SHA.fullmatch(config.head_sha):
        raise GateError("head SHA must be a full 40-character hexadecimal SHA")
    repo = config.repo.resolve()
    origin_url = run(runner, ["git", "-C", str(repo), "remote", "get-url", "origin"]).strip()
    if normalize_remote(origin_url).casefold() != config.github_repo.casefold():
        raise GateError("local origin does not match --github-repo")

    pr_payload = object_value(
        run_json(
            runner,
            ["gh", "pr", "view", str(config.pr), "--repo", config.github_repo, "--json", "number,state,headRefName,headRefOid,body,url"],
            "current PR query",
        ),
        "current PR query",
    )
    if positive_int(pr_payload, "number", "PR") != config.pr or text_field(pr_payload, "state", "PR").upper() != "OPEN":
        raise GateError("current implementation PR is not the requested open PR")
    head_ref = text_field(pr_payload, "headRefName", "PR")
    if text_field(pr_payload, "headRefOid", "PR").casefold() != config.head_sha.casefold():
        raise GateError("current implementation PR head does not match --head-sha")
    pr_body = text_field(pr_payload, "body", "PR")
    if not references_issue(pr_body, config.issue):
        raise GateError("current implementation PR body does not reference the linked issue")

    label_event, label_authority = latest_ready_event(runner, config.github_repo, config.issue)
    with tempfile.TemporaryDirectory(prefix="remem-sensitive-gate-") as temp_name:
        temp = Path(temp_name)
        issue_path = temp / "issue-evidence.json"
        original_duplicate_path = temp / "duplicate-original.json"
        filtered_duplicate_path = temp / "duplicate-filtered.json"
        issue_evidence = object_value(
            run_json(runner, [sys.executable, str(repo / "checks/github_issue_evidence.py"), "--repo", str(repo), "--github-repo", config.github_repo, "--issue", str(config.issue), "--json"], "issue evidence collector"),
            "issue evidence collector",
        )
        if issue_evidence.get("repository") != config.github_repo or issue_evidence.get("issue") != config.issue:
            raise GateError("issue evidence repository/issue binding failed")
        if issue_evidence.get("state") != READY_LABEL or issue_evidence.get("state_source") != "label" or issue_evidence.get("state_trusted") is not True:
            raise GateError("issue evidence is not trusted label-derived ready_to_implement state")
        duplicate = validate_duplicate(
            run_json(runner, [sys.executable, str(repo / "checks/github_duplicate_evidence.py"), "--github-repo", config.github_repo, "--issue", str(config.issue), "--remote", "origin", "--pr-limit", str(config.pr_limit), "--json"], "duplicate evidence collector"),
            config.issue,
        )
        age = check_freshness(duplicate, started)
        conflicts = [branch for branch in matching_branches(repo, config.issue, duplicate["remote_branches"]) if branch != head_ref]
        decision, decided_branches = ownership_decision(runner, config, pr_body, conflicts)
        filtered, exemptions = filter_duplicates(duplicate, config, head_ref, decided_branches)
        issue_path.write_bytes(canonical_bytes(issue_evidence))
        original_duplicate_path.write_bytes(canonical_bytes(duplicate))
        filtered_duplicate_path.write_bytes(canonical_bytes(filtered))
        issue_pre = file_hash(issue_path)
        duplicate_pre = file_hash(filtered_duplicate_path)
        route_argv = [sys.executable, str(repo / "checks/route_gate.py"), "--repo", str(repo), "--route", "implement", "--issue", str(config.issue), "--mode", "required", "--evidence", str(issue_path), "--duplicate-evidence", str(filtered_duplicate_path), "--json"]
        if "--state" in route_argv or "--label" in route_argv:
            raise GateError("fixed route argv contains a forbidden state/label override")
        route = validate_route(run_json(runner, route_argv, "route gate"))
        issue_post = file_hash(issue_path)
        duplicate_post = file_hash(filtered_duplicate_path)
        if issue_pre != issue_post or duplicate_pre != duplicate_post:
            raise GateError("route input evidence changed during gate execution")

    completed = now().astimezone(timezone.utc)
    result = {
        "schema_version": 1,
        "wrapper": WRAPPER_ID,
        "started_at": iso8601(started),
        "completed_at": iso8601(completed),
        "repository": config.github_repo,
        "remote_url": origin_url,
        "issue": config.issue,
        "pr": {"number": config.pr, "url": text_field(pr_payload, "url", "PR"), "state": "OPEN", "head_ref": head_ref, "head_sha": config.head_sha},
        "label_event": label_event,
        "label_authority": label_authority,
        "evidence_trust": {"state": READY_LABEL, "state_source": "label", "state_trusted": True, "duplicate_age_seconds": age, "max_age_seconds": MAX_EVIDENCE_AGE_SECONDS, "fresh": True},
        "branch_ownership_decision": decision,
        "self_exemptions": exemptions,
        "artifacts": {"issue_evidence": issue_evidence, "duplicate_original": duplicate, "duplicate_filtered": filtered},
        "artifact_hashes": {
            "issue_evidence_pre": issue_pre,
            "issue_evidence_post": issue_post,
            "duplicate_filtered_pre": duplicate_pre,
            "duplicate_filtered_post": duplicate_post,
            "duplicate_original": value_hash(duplicate),
            "original_open_prs": value_hash(duplicate["open_prs"]),
            "filtered_open_prs": value_hash(filtered["open_prs"]),
            "original_remote_branches": value_hash(duplicate["remote_branches"]),
            "filtered_remote_branches": value_hash(filtered["remote_branches"]),
        },
        "route_argv": route_argv,
        "route_gate": route,
        "decision": "allowed",
    }
    sys.path.insert(0, str(repo / "checks"))
    try:
        from specrail_lib import validate_instance
        schema = json.loads((repo / "schemas/sensitive_implement_gate_result.schema.json").read_text(encoding="utf-8"))
        validate_instance(schema, result)
    except Exception as exc:
        raise GateError(f"durable wrapper result failed schema validation: {exc}") from exc
    return result


def parse_args() -> Config:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", default=".")
    parser.add_argument("--github-repo", required=True)
    parser.add_argument("--issue", required=True, type=int)
    parser.add_argument("--pr", required=True, type=int)
    parser.add_argument("--head-sha", required=True)
    parser.add_argument("--output", required=True)
    parser.add_argument("--pr-limit", type=int, default=100)
    args = parser.parse_args()
    return Config(Path(args.repo), args.github_repo, args.issue, args.pr, args.head_sha.lower(), Path(args.output), args.pr_limit)


def main() -> int:
    config = parse_args()
    try:
        result = execute(config)
        config.output.parent.mkdir(parents=True, exist_ok=True)
        config.output.write_bytes(canonical_bytes(result))
    except (GateError, OSError, json.JSONDecodeError) as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 1
    print(json.dumps(result, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    sys.exit(main())
