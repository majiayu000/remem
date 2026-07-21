#!/usr/bin/env python3
"""Focused offline tests for the GH-813 sensitive implementation wrapper."""

from __future__ import annotations

import importlib.util
import json
import subprocess
import sys
import unittest
from copy import deepcopy
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


REPO = Path(__file__).resolve().parents[2]
MODULE_PATH = REPO / "scripts/ci/run_sensitive_implement_gate.py"
SPEC = importlib.util.spec_from_file_location("sensitive_gate", MODULE_PATH)
assert SPEC is not None and SPEC.loader is not None
gate = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = gate
SPEC.loader.exec_module(gate)

NOW = datetime(2026, 7, 21, 12, 0, tzinfo=timezone.utc)
HEAD = "a" * 40
REPO_NAME = "majiayu000/remem"
HEAD_REF = "codex/gh813-implementation"


def completed(argv: list[str], payload: Any = None, *, stdout: str | None = None) -> subprocess.CompletedProcess[str]:
    text = stdout if stdout is not None else json.dumps(payload)
    return subprocess.CompletedProcess(argv, 0, text, "")


class FakeRunner:
    def __init__(self) -> None:
        self.origin = "git@github.com:majiayu000/remem.git"
        self.pr = {
            "number": 908,
            "state": "OPEN",
            "headRefName": HEAD_REF,
            "headRefOid": HEAD,
            "body": "Implements GH-813",
            "url": "https://github.com/majiayu000/remem/pull/908",
        }
        self.events: Any = [[{
            "id": 17,
            "event": "labeled",
            "created_at": "2026-07-21T11:59:00Z",
            "label": {"name": "ready_to_implement"},
            "actor": {"login": "majiayu000", "type": "User"},
        }]]
        self.issue = {
            "issue": 813,
            "repository": REPO_NAME,
            "state": "ready_to_implement",
            "state_source": "label",
            "state_trusted": True,
        }
        self.duplicate = {
            "issue": 813,
            "collected_at": "2026-07-21T11:59:30Z",
            "open_prs_complete": True,
            "open_pr_limit": 100,
            "open_prs": [{"number": 908, "head_ref": HEAD_REF, "references_issue": True}],
            "remote_branches": [HEAD_REF, "main"],
        }
        self.permission = {"permission": "maintain", "user": {"type": "User"}}
        self.comment: dict[str, Any] | None = None
        self.mutate_route_input = False
        self.commands: list[list[str]] = []

    def __call__(self, argv: list[str]) -> subprocess.CompletedProcess[str]:
        self.commands.append(list(argv))
        if argv[:4] == ["git", "-C", str(REPO), "remote"]:
            return completed(argv, stdout=self.origin + "\n")
        if argv[:3] == ["gh", "pr", "view"]:
            return completed(argv, self.pr)
        if argv[:4] == ["gh", "api", "--paginate", "--slurp"]:
            return completed(argv, self.events)
        if argv[:2] == ["gh", "api"] and "/collaborators/" in argv[2]:
            return completed(argv, self.permission)
        if argv[:2] == ["gh", "api"] and "/issues/comments/" in argv[2]:
            return completed(argv, self.comment)
        if argv[0] == sys.executable and argv[1].endswith("github_issue_evidence.py"):
            return completed(argv, self.issue)
        if argv[0] == sys.executable and argv[1].endswith("github_duplicate_evidence.py"):
            return completed(argv, self.duplicate)
        if argv[0] == sys.executable and argv[1].endswith("route_gate.py"):
            evidence_path = Path(argv[argv.index("--evidence") + 1])
            duplicate_path = Path(argv[argv.index("--duplicate-evidence") + 1])
            duplicate = json.loads(duplicate_path.read_text())
            blocked = any(item["references_issue"] for item in duplicate["open_prs"])
            matching = gate.matching_branches(REPO, 813, duplicate["remote_branches"])
            if self.mutate_route_input:
                evidence_path.write_text("{}\n", encoding="utf-8")
            result = {
                "decision": "blocked" if blocked or matching else "allowed",
                "missing": ["duplicate_work"] if blocked or matching else [],
                "current_state": "ready_to_implement",
                "satisfied": ["state provided by evidence: ready_to_implement (label)"],
                "sensitive_classification": {"enforcement_sensitive": True},
            }
            return completed(argv, result)
        raise AssertionError(f"unexpected command: {argv}")


def config() -> Any:
    return gate.Config(REPO, REPO_NAME, 813, 908, HEAD, REPO / ".specrail/unused.json")


class SensitiveImplementGateTests(unittest.TestCase):
    def execute(self, runner: FakeRunner) -> dict[str, Any]:
        return gate.execute(config(), runner=runner, now=lambda: NOW)

    def assert_blocked(self, runner: FakeRunner, pattern: str) -> None:
        with self.assertRaisesRegex(gate.GateError, pattern):
            self.execute(runner)

    def test_schema_valid_current_head_positive(self) -> None:
        runner = FakeRunner()
        result = self.execute(runner)
        self.assertEqual(result["decision"], "allowed")
        self.assertEqual(result["pr"]["head_sha"], HEAD)
        self.assertEqual(result["label_authority"]["role"], "owner")
        self.assertEqual(result["artifact_hashes"]["issue_evidence_pre"], result["artifact_hashes"]["issue_evidence_post"])
        self.assertEqual(result["artifact_hashes"]["duplicate_filtered_pre"], result["artifact_hashes"]["duplicate_filtered_post"])
        route = next(command for command in runner.commands if command[1].endswith("route_gate.py"))
        self.assertNotIn("--state", route)
        self.assertNotIn("--label", route)
        self.assertEqual(result["artifacts"]["duplicate_original"]["open_prs"][0]["number"], 908)
        self.assertEqual(result["artifacts"]["duplicate_filtered"]["open_prs"], [])

    def test_wrong_local_origin_fails(self) -> None:
        runner = FakeRunner()
        runner.origin = "https://github.com/other/remem.git"
        self.assert_blocked(runner, "origin does not match")

    def test_wrong_pr_head_fails(self) -> None:
        runner = FakeRunner()
        runner.pr["headRefOid"] = "b" * 40
        self.assert_blocked(runner, "head does not match")

    def test_closed_pr_fails(self) -> None:
        runner = FakeRunner()
        runner.pr["state"] = "MERGED"
        self.assert_blocked(runner, "not the requested open PR")

    def test_unlinked_issue_fails(self) -> None:
        runner = FakeRunner()
        runner.pr["body"] = "No linked work"
        self.assert_blocked(runner, "does not reference")

    def test_bot_label_actor_fails(self) -> None:
        runner = FakeRunner()
        runner.events[0][0]["actor"] = {"login": "release[bot]", "type": "Bot"}
        self.assert_blocked(runner, "non-bot")

    def test_agent_named_user_fails(self) -> None:
        runner = FakeRunner()
        runner.events[0][0]["actor"] = {"login": "merge-agent", "type": "User"}
        self.assert_blocked(runner, "non-bot")

    def test_non_maintainer_label_actor_fails(self) -> None:
        runner = FakeRunner()
        runner.events[0][0]["actor"] = {"login": "contributor", "type": "User"}
        runner.permission["permission"] = "write"
        self.assert_blocked(runner, "lacks live admin or maintain")

    def test_missing_label_event_actor_fails(self) -> None:
        runner = FakeRunner()
        del runner.events[0][0]["actor"]
        self.assert_blocked(runner, "label event actor")

    def test_untrusted_issue_label_state_fails(self) -> None:
        runner = FakeRunner()
        runner.issue["state_trusted"] = False
        self.assert_blocked(runner, "not trusted label-derived")

    def test_stale_duplicate_evidence_fails(self) -> None:
        runner = FakeRunner()
        runner.duplicate["collected_at"] = "2026-07-21T11:54:59Z"
        self.assert_blocked(runner, "older than 300")

    def test_future_duplicate_evidence_fails(self) -> None:
        runner = FakeRunner()
        runner.duplicate["collected_at"] = "2026-07-21T12:00:01Z"
        self.assert_blocked(runner, "future-dated")

    def test_incomplete_pr_collection_fails(self) -> None:
        runner = FakeRunner()
        runner.duplicate["open_prs_complete"] = False
        self.assert_blocked(runner, "incomplete open PR")

    def test_other_referencing_pr_remains_blocking(self) -> None:
        runner = FakeRunner()
        runner.duplicate["open_prs"].append({"number": 777, "head_ref": "human/other", "references_issue": True})
        self.assert_blocked(runner, "nested route gate is not allowed")

    def test_other_matching_branch_requires_decision(self) -> None:
        runner = FakeRunner()
        runner.duplicate["remote_branches"].append("human/gh813-existing")
        self.assert_blocked(runner, "exactly one explicit SpecRail")

    def test_human_branch_decision_filters_only_listed_branch(self) -> None:
        runner = FakeRunner()
        runner.duplicate["remote_branches"].append("human/gh813-existing")
        runner.pr["body"] += "\nhttps://github.com/majiayu000/remem/issues/813#issuecomment-12345"
        runner.comment = {
            "body": "\n".join([
                "SpecRail branch ownership decision",
                "Repository: majiayu000/remem", "Issue: 813", "PR: 908", f"Head-SHA: {HEAD}",
                "Decision: continue_existing_work", "Rationale: maintainer confirms this is the same implementation lane",
                "Branches:", "- human/gh813-existing",
            ]),
            "created_at": "2026-07-21T11:58:00Z",
            "html_url": "https://github.com/majiayu000/remem/issues/813#issuecomment-12345",
            "user": {"login": "majiayu000", "type": "User"},
        }
        result = self.execute(runner)
        self.assertEqual(result["branch_ownership_decision"]["branches"], ["human/gh813-existing"])
        self.assertNotIn("human/gh813-existing", result["artifacts"]["duplicate_filtered"]["remote_branches"])
        self.assertIn("main", result["artifacts"]["duplicate_filtered"]["remote_branches"])

    def test_decision_must_cover_every_conflicting_branch(self) -> None:
        runner = FakeRunner()
        runner.duplicate["remote_branches"] += ["human/gh813-one", "human/gh813-two"]
        runner.pr["body"] += "\nhttps://github.com/majiayu000/remem/issues/813#issuecomment-12345"
        runner.comment = {
            "body": "\n".join([
                "SpecRail branch ownership decision",
                "Repository: majiayu000/remem", "Issue: 813", "PR: 908", f"Head-SHA: {HEAD}",
                "Decision: continue_existing_work", "Rationale: only one branch was reviewed", "Branches: human/gh813-one",
            ]),
            "created_at": "2026-07-21T11:58:00Z",
            "html_url": "https://github.com/majiayu000/remem/issues/813#issuecomment-12345",
            "user": {"login": "majiayu000", "type": "User"},
        }
        self.assert_blocked(runner, "does not cover all conflicting")

    def test_sha_authorization_comment_can_coexist_with_ownership_decision(self) -> None:
        runner = FakeRunner()
        runner.duplicate["remote_branches"].append("human/gh813-existing")
        runner.pr["body"] += "\n".join([
            "\nhttps://github.com/majiayu000/remem/issues/813#issuecomment-99999",
            "https://github.com/majiayu000/remem/issues/813#issuecomment-12345",
        ])
        ownership = {
            "body": "\n".join([
                "SpecRail branch ownership decision",
                "Repository: majiayu000/remem", "Issue: 813", "PR: 908", f"Head-SHA: {HEAD}",
                "Decision: continue_existing_work", "Rationale: one implementation lane is retained",
                "Branches: human/gh813-existing",
            ]),
            "created_at": "2026-07-21T11:58:00Z",
            "html_url": "https://github.com/majiayu000/remem/issues/813#issuecomment-12345",
            "user": {"login": "majiayu000", "type": "User"},
        }
        authorization = {
            "body": "Exact-SHA authorization only",
            "created_at": "2026-07-21T11:57:00Z",
            "html_url": "https://github.com/majiayu000/remem/issues/813#issuecomment-99999",
            "user": {"login": "majiayu000", "type": "User"},
        }
        original = runner.__call__

        def comments(argv: list[str]) -> subprocess.CompletedProcess[str]:
            if argv[:2] == ["gh", "api"] and argv[2].endswith("/99999"):
                return completed(argv, authorization)
            if argv[:2] == ["gh", "api"] and argv[2].endswith("/12345"):
                return completed(argv, ownership)
            return original(argv)

        result = gate.execute(config(), runner=comments, now=lambda: NOW)
        self.assertEqual(result["branch_ownership_decision"]["comment_id"], 12345)

    def test_ci_invokes_wrapper_and_never_bare_route_gate(self) -> None:
        workflow = (REPO / ".github/workflows/ci.yml").read_text(encoding="utf-8")
        self.assertIn("scripts/ci/test_run_sensitive_implement_gate.py", workflow)
        self.assertIn("scripts/ci/run_sensitive_implement_gate.py", workflow)
        self.assertNotIn("python3 checks/route_gate.py", workflow)

    def test_gate_input_hash_drift_fails(self) -> None:
        runner = FakeRunner()
        runner.mutate_route_input = True
        self.assert_blocked(runner, "evidence changed during gate")

    def test_duplicate_schema_extra_field_fails(self) -> None:
        runner = FakeRunner()
        runner.duplicate["repository"] = REPO_NAME
        self.assert_blocked(runner, "fields do not match")


if __name__ == "__main__":
    unittest.main()
