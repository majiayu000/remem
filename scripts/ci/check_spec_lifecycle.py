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


AUTO_CLOSE_RE = re.compile(
    r"\b(?:close[sd]?|fix(?:e[sd])?|resolve[sd]?)\b\s*:?\s+"
    r"(?:[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+)?#\d+\b",
    re.I,
)
REFS_RE = re.compile(r"\brefs?\b\s*:?\s+(?:[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+)?#\d+\b", re.I)
NO_ISSUE_RE = re.compile(r"\bno issue:\s*\S+", re.I)
API_DOCS_NOT_NEEDED_RE = re.compile(
    r"^\s*API contract docs:\s*not needed\s*[-:]\s+\S.+$",
    re.I | re.M,
)
PR_TYPES = ("Spec only", "Implementation", "Bugfix", "Release/docs/process")


def file_exists_at_ref(path: str, ref: str) -> bool:
    if ref == "WORKTREE":
        return Path(path).exists()
    result = subprocess.run(
        ["git", "cat-file", "-e", f"{ref}:{path}"],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        text=True,
    )
    return result.returncode == 0


def file_text_at_ref(path: str, ref: str) -> str:
    if ref == "WORKTREE":
        p = Path(path)
        return p.read_text(encoding="utf-8") if p.exists() else ""
    result = subprocess.run(
        ["git", "show", f"{ref}:{path}"],
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
        text=True,
    )
    return result.stdout if result.returncode == 0 else ""


def pr_body_from_env() -> str:
    body = os.environ.get("GITHUB_PR_BODY", "")
    if body:
        return body

    event_path = os.environ.get("GITHUB_EVENT_PATH")
    if event_path and Path(event_path).exists():
        with open(event_path, "r", encoding="utf-8") as fh:
            event = json.load(fh)
        pr = event.get("pull_request") or {}
        return pr.get("body") or ""

    return ""


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


def spec_id_indexed(spec_id: str, head: str) -> bool:
    readme = file_text_at_ref("docs/specs/README.md", head)
    return f"`{spec_id}/`" in readme or f"docs/specs/{spec_id}/" in readme


def check_lifecycle(
    changes: list[str], body: str, head: str = "HEAD", base: str = "HEAD^"
) -> list[str]:
    failures: list[str] = []

    checked_types = [label for label in PR_TYPES if checked(label, body)]
    is_spec_only = "Spec only" in checked_types
    spec_dirs = spec_dirs_added_or_changed(changes)
    touches_src = has_prefix(changes, "src/")
    lifecycle_relevant = bool(spec_dirs) or touches_src

    if lifecycle_relevant and len(checked_types) != 1:
        failures.append(
            "Lifecycle-relevant PRs must select exactly one PR Type checkbox."
        )

    if is_spec_only:
        if AUTO_CLOSE_RE.search(body):
            failures.append(
                "Spec-only PRs must use Refs, not Closes/Fixes/Resolves. "
                "Replace the auto-close wording and create/link implementation issue(s)."
            )
        if not REFS_RE.search(body):
            failures.append("Spec-only PRs must include at least one `Refs #...` link.")

    for spec_id in sorted(spec_dirs):
        product = f"docs/specs/{spec_id}/PRODUCT.md"
        tech = f"docs/specs/{spec_id}/TECH.md"
        if not file_exists_at_ref(product, head) or not file_exists_at_ref(tech, head):
            failures.append(
                f"Current spec `{spec_id}` must include both PRODUCT.md and TECH.md."
            )
        is_new_spec = not file_exists_at_ref(product, base) and not file_exists_at_ref(tech, base)
        if is_new_spec and not has_path(changes, "docs/specs/README.md"):
            failures.append(
                f"New current spec `{spec_id}` must update docs/specs/README.md."
            )
        if is_new_spec and not spec_id_indexed(spec_id, head):
            failures.append(f"New current spec `{spec_id}` must be indexed in docs/specs/README.md.")

    if touches_src:
        if is_spec_only:
            failures.append("PRs that touch src/** cannot be marked Spec only.")
        if not AUTO_CLOSE_RE.search(body) and not NO_ISSUE_RE.search(body):
            failures.append(
                "PRs that touch src/** must include `Closes #...` "
                "or an explicit `No issue: ...` explanation."
            )

    if has_prefix(changes, "src/api/"):
        if not has_path(changes, "docs/specs/SPEC-web-api.md") and not API_DOCS_NOT_NEEDED_RE.search(body):
            failures.append(
                "PRs touching src/api/** must update docs/specs/SPEC-web-api.md "
                "or include `API contract docs: not needed - <reason>` in the PR body."
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
            ["docs/specs/missingreadme/PRODUCT.md", "docs/specs/missingreadme/TECH.md"],
            "- [x] Spec only\n\nRefs #123",
            {"must update docs/specs/README.md", "must be indexed"},
        ),
        (
            "spec closes fails",
            ["docs/specs/demo/PRODUCT.md"],
            "- [x] Spec only\n\nCloses #123",
            {"Spec-only PRs must use Refs"},
        ),
        (
            "spec closes colon fails",
            ["docs/specs/demo/PRODUCT.md", "docs/specs/demo/TECH.md", "docs/specs/README.md"],
            "- [x] Spec only\n\nRefs #123\n\nCloses: #123",
            {"Spec-only PRs must use Refs"},
        ),
        (
            "spec cross repo closes fails",
            ["docs/specs/demo/PRODUCT.md", "docs/specs/demo/TECH.md", "docs/specs/README.md"],
            "- [x] Spec only\n\nRefs #123\n\nFixes majiayu000/remem#123",
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
            {"PRs that touch src/**"},
        ),
        (
            "implementation title-only close fails",
            ["src/lib.rs"],
            "- [x] Implementation",
            {"PRs that touch src/**"},
        ),
        (
            "bugfix src missing issue fails",
            ["src/lib.rs"],
            "- [x] Bugfix",
            {"PRs that touch src/**"},
        ),
        (
            "bugfix src no issue passes",
            ["src/lib.rs"],
            "- [x] Bugfix\n\nNo issue: emergency local-only diagnostic repair",
            set(),
        ),
        (
            "release docs process src missing issue fails",
            ["src/lib.rs"],
            "- [x] Release/docs/process",
            {"PRs that touch src/**"},
        ),
        (
            "spec only src fails",
            ["src/lib.rs"],
            "- [x] Spec only\n\nRefs #123",
            {"cannot be marked Spec only", "PRs that touch src/**"},
        ),
        (
            "api docs marker passes",
            ["src/api/server.rs"],
            "- [x] Implementation\n\nCloses #123\n\nAPI contract docs: not needed - internal refactor only",
            set(),
        ),
        (
            "api default template wording fails",
            ["src/api/server.rs"],
            "- [x] Implementation\n\nCloses #123\n\n- [ ] If touching `src/api/**`, updated `docs/specs/SPEC-web-api.md` or wrote an explicit API docs waiver with rationale",
            {"PRs touching src/api/**"},
        ),
        (
            "api docs missing fails",
            ["src/api/server.rs"],
            "- [x] Implementation\n\nCloses #123",
            {"PRs touching src/api/**"},
        ),
        (
            "unchecked lifecycle type fails",
            ["docs/specs/demo/PRODUCT.md", "docs/specs/demo/TECH.md", "docs/specs/README.md"],
            "- [ ] Spec only\n- [ ] Implementation\n\nCloses #123",
            {"must select exactly one PR Type"},
        ),
        (
            "existing spec update does not require readme",
            ["docs/specs/existing/TECH.md"],
            "- [x] Spec only\n\nRefs #123",
            set(),
        ),
        (
            "new spec readme must mention spec id",
            ["docs/specs/unindexed/PRODUCT.md", "docs/specs/unindexed/TECH.md", "docs/specs/README.md"],
            "- [x] Spec only\n\nRefs #123",
            {"must be indexed"},
        ),
    ]

    original_file_exists = globals()["file_exists_at_ref"]
    original_file_text = globals()["file_text_at_ref"]

    existing_base = {
        "docs/specs/existing/PRODUCT.md",
        "docs/specs/existing/TECH.md",
    }

    def fake_file_exists(path: str, ref: str = "HEAD") -> bool:
        if ref == "BASE":
            return path in existing_base
        return not path.startswith("docs/specs/missing/")

    def fake_file_text(path: str, ref: str = "HEAD") -> str:
        if path != "docs/specs/README.md":
            return ""
        return "`demo/`\n`existing/`\n"

    globals()["file_exists_at_ref"] = fake_file_exists
    globals()["file_text_at_ref"] = fake_file_text
    try:
        for name, changes, body, expected_fragments in cases:
            failures = check_lifecycle(changes, body, head="HEAD", base="BASE")
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
        globals()["file_exists_at_ref"] = original_file_exists
        globals()["file_text_at_ref"] = original_file_text

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
    failures = check_lifecycle(changes, body, args.head, args.base)
    if failures:
        print("Spec lifecycle check failed:", file=sys.stderr)
        for failure in failures:
            print(f"- {failure}", file=sys.stderr)
        return 1

    print("Spec lifecycle check passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
