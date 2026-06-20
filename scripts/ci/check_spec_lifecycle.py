#!/usr/bin/env python3
"""Check remem spec/implementation PR lifecycle rules."""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
from pathlib import Path

from check_version_bump import changed_files as git_changed_files


AUTO_CLOSE_RE = re.compile(r"\b(?:close[sd]?|fix(?:e[sd])?|resolve[sd]?)\s+#\d+", re.I)
REFS_RE = re.compile(r"\brefs?\s+#\d+", re.I)
NO_ISSUE_RE = re.compile(r"\bno issue:\s*\S+", re.I)
API_DOCS_NOT_NEEDED_RE = re.compile(r"\bAPI contract docs:\s*not needed\b", re.I)


def file_exists_at_head(path: str, head: str) -> bool:
    if head == "WORKTREE":
        return Path(path).exists()
    result = subprocess.run(
        ["git", "cat-file", "-e", f"{head}:{path}"],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        text=True,
    )
    return result.returncode == 0


def pr_body_from_env() -> str:
    title = os.environ.get("GITHUB_PR_TITLE", "")
    body = os.environ.get("GITHUB_PR_BODY", "")
    if body:
        return f"{title}\n\n{body}"

    event_path = os.environ.get("GITHUB_EVENT_PATH")
    if event_path and Path(event_path).exists():
        with open(event_path, "r", encoding="utf-8") as fh:
            event = json.load(fh)
        pr = event.get("pull_request") or {}
        return f"{pr.get('title') or title}\n\n{pr.get('body') or ''}"

    return title


def checked(label: str, body: str) -> bool:
    pattern = re.compile(rf"^\s*[-*]\s+\[[xX]\]\s+{re.escape(label)}\s*$", re.M)
    return pattern.search(body) is not None


def spec_dirs_added_or_changed(changes: list[str]) -> set[str]:
    dirs: set[str] = set()
    for path in changes:
        parts = path.split("/")
        if len(parts) == 4 and parts[0] == "docs" and parts[1] == "specs":
            spec_id = parts[2]
            leaf = parts[3]
            if spec_id != "refactor-steps" and leaf in {"PRODUCT.md", "TECH.md"}:
                dirs.add(spec_id)
    return dirs


def has_path(changes: list[str], path: str) -> bool:
    return path in changes


def has_prefix(changes: list[str], prefix: str) -> bool:
    return any(path.startswith(prefix) for path in changes)


def check_lifecycle(changes: list[str], body: str, head: str = "HEAD") -> list[str]:
    failures: list[str] = []

    is_spec_only = checked("Spec only", body)
    is_implementation = checked("Implementation", body)

    if is_spec_only:
        if AUTO_CLOSE_RE.search(body):
            failures.append(
                "Spec-only PRs must use Refs, not Closes/Fixes/Resolves. "
                "Replace the auto-close wording and create/link implementation issue(s)."
            )
        if not REFS_RE.search(body):
            failures.append("Spec-only PRs must include at least one `Refs #...` link.")

    for spec_id in sorted(spec_dirs_added_or_changed(changes)):
        product = f"docs/specs/{spec_id}/PRODUCT.md"
        tech = f"docs/specs/{spec_id}/TECH.md"
        if not file_exists_at_head(product, head) or not file_exists_at_head(tech, head):
            failures.append(
                f"Current spec `{spec_id}` must include both PRODUCT.md and TECH.md."
            )
        if not has_path(changes, "docs/specs/README.md"):
            failures.append(
                f"Current spec `{spec_id}` changed without updating docs/specs/README.md."
            )

    if is_implementation and has_prefix(changes, "src/"):
        if not AUTO_CLOSE_RE.search(body) and not NO_ISSUE_RE.search(body):
            failures.append(
                "Implementation PRs that touch src/** must include `Closes #...` "
                "or an explicit `No issue: ...` explanation."
            )

    if has_prefix(changes, "src/api/"):
        if not has_path(changes, "docs/specs/SPEC-web-api.md") and not API_DOCS_NOT_NEEDED_RE.search(body):
            failures.append(
                "PRs touching src/api/** must update docs/specs/SPEC-web-api.md "
                "or include `API contract docs: not needed` in the PR body."
            )

    return failures


def self_test() -> int:
    cases = [
        (
            "spec refs passes",
            ["docs/specs/demo/PRODUCT.md", "docs/specs/demo/TECH.md", "docs/specs/README.md"],
            "- [x] Spec only\n\nRefs #123",
            set(),
        ),
        (
            "new spec missing readme fails",
            ["docs/specs/demo/PRODUCT.md", "docs/specs/demo/TECH.md"],
            "- [ ] Spec only\n\nRefs #123",
            {"changed without updating docs/specs/README.md"},
        ),
        (
            "spec closes fails",
            ["docs/specs/demo/PRODUCT.md"],
            "- [x] Spec only\n\nCloses #123",
            {"Spec-only PRs must use Refs"},
        ),
        (
            "implementation src closes passes",
            ["src/lib.rs"],
            "- [x] Implementation\n\nCloses #123",
            set(),
        ),
        (
            "implementation src missing issue fails",
            ["src/lib.rs"],
            "- [x] Implementation",
            {"Implementation PRs that touch src/**"},
        ),
        (
            "api docs marker passes",
            ["src/api/server.rs"],
            "- [x] Implementation\n\nCloses #123\n\nAPI contract docs: not needed",
            set(),
        ),
        (
            "api docs missing fails",
            ["src/api/server.rs"],
            "- [x] Implementation\n\nCloses #123",
            {"PRs touching src/api/**"},
        ),
    ]

    original_file_exists = globals()["file_exists_at_head"]
    globals()["file_exists_at_head"] = lambda path, head="HEAD": not path.startswith("docs/specs/missing/")
    try:
        for name, changes, body, expected_fragments in cases:
            failures = check_lifecycle(changes, body)
            text = "\n".join(failures)
            if expected_fragments:
                for fragment in expected_fragments:
                    if fragment not in text:
                        print(f"self-test failed: {name}: missing {fragment!r}", file=sys.stderr)
                        print(text, file=sys.stderr)
                        return 1
            elif failures:
                print(f"self-test failed: {name}: unexpected failures", file=sys.stderr)
                print(text, file=sys.stderr)
                return 1
    finally:
        globals()["file_exists_at_head"] = original_file_exists

    print("check_spec_lifecycle self-test passed")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("base", nargs="?")
    parser.add_argument("head", nargs="?", default="HEAD")
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()

    if args.self_test:
        return self_test()
    if not args.base:
        parser.error("base revision is required unless --self-test is used")

    changes = git_changed_files(args.base, args.head)
    body = pr_body_from_env()
    failures = check_lifecycle(changes, body, args.head)
    if failures:
        print("Spec lifecycle check failed:", file=sys.stderr)
        for failure in failures:
            print(f"- {failure}", file=sys.stderr)
        return 1

    print("Spec lifecycle check passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
