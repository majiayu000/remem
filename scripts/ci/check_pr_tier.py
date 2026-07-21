#!/usr/bin/env python3
"""Validate declared PR tier claims against the actual diff.

PR tiers (`Tier: fastlane|standard|heavy` in the PR body) let small changes
skip the separate-spec-PR process. The tier is only trustworthy if a
machine, not the PR author, verifies the claim. PRs without a `Tier:` line
are unaffected.
"""

from __future__ import annotations

import argparse
import re
import subprocess
import sys

from check_spec_lifecycle import pr_body_from_env

TIER_RE = re.compile(r"^\s*Tier:\s*(\S+)\s*$", re.I | re.M)
KNOWN_TIERS = ("fastlane", "standard", "heavy")

FASTLANE_MAX_LINES = 50
# Paths where a change is never fastlane: API surface, CI definitions,
# install/config plumbing, and schema/migration files.
PROTECTED_PREFIXES = (
    "src/api/",
    "src/install/",
    ".github/workflows/",
)
PROTECTED_NAME_RE = re.compile(r"(schema|migration)", re.I)
# Docs and specs never count against the fastlane line budget.
DOC_EXEMPT_RE = re.compile(r"^docs/|\.md$")


def diff_numstat(base: str, head: str) -> list[tuple[str, str, str]]:
    result = subprocess.run(
        ["git", "diff", "--numstat", f"{base}..{head}"],
        stdout=subprocess.PIPE,
        check=True,
        text=True,
    )
    rows = []
    for line in result.stdout.splitlines():
        parts = line.split("\t")
        if len(parts) == 3:
            rows.append((parts[0], parts[1], parts[2]))
    return rows


def declared_tier(body: str) -> str | None:
    match = TIER_RE.search(body)
    return match.group(1).lower() if match else None


def check_tier(body: str, numstat: list[tuple[str, str, str]]) -> list[str]:
    tier = declared_tier(body)
    if tier is None:
        return []
    if tier not in KNOWN_TIERS:
        return [f"Unknown tier `{tier}`; use one of: {', '.join(KNOWN_TIERS)}."]
    if tier != "fastlane":
        return []

    failures: list[str] = []
    code_lines = 0
    for added, deleted, path in numstat:
        if any(path.startswith(p) for p in PROTECTED_PREFIXES) or (
            path.startswith("src/") and PROTECTED_NAME_RE.search(path)
        ):
            failures.append(
                f"Fastlane PRs must not touch protected path `{path}`; "
                "use Tier: standard or heavy."
            )
        if DOC_EXEMPT_RE.search(path):
            continue
        if added == "-" or deleted == "-":
            failures.append(
                f"Fastlane PRs must not change binary file `{path}`."
            )
            continue
        code_lines += int(added) + int(deleted)

    if code_lines > FASTLANE_MAX_LINES:
        failures.append(
            f"Fastlane PRs are limited to {FASTLANE_MAX_LINES} changed "
            f"non-doc lines; this PR changes {code_lines}."
        )
    return failures


def self_test() -> int:
    cases = [
        (
            "no tier line passes",
            "Closes #1",
            [("100", "50", "src/big.rs")],
            set(),
        ),
        (
            "unknown tier fails",
            "Tier: turbo",
            [("1", "1", "src/lib.rs")],
            {"Unknown tier"},
        ),
        (
            "fastlane small change passes",
            "Tier: fastlane\nCloses #1",
            [("20", "10", "src/context.rs"), ("300", "0", "docs/notes.md")],
            set(),
        ),
        (
            "fastlane over budget fails",
            "Tier: fastlane",
            [("40", "20", "src/context.rs")],
            {"limited to 50"},
        ),
        (
            "fastlane protected api path fails",
            "Tier: fastlane",
            [("5", "5", "src/api/handlers.rs")],
            {"protected path"},
        ),
        (
            "fastlane workflow path fails",
            "Tier: fastlane",
            [("2", "2", ".github/workflows/ci.yml")],
            {"protected path"},
        ),
        (
            "fastlane schema name fails",
            "Tier: fastlane",
            [("3", "3", "src/store/schema_v70.rs")],
            {"protected path"},
        ),
        (
            "fastlane binary fails",
            "Tier: fastlane",
            [("-", "-", "assets/logo.png")],
            {"binary file"},
        ),
        (
            "standard tier passes untouched",
            "Tier: standard",
            [("500", "200", "src/api/handlers.rs")],
            set(),
        ),
        (
            "case-insensitive tier line",
            "tier: FASTLANE",
            [("10", "0", "src/lib.rs")],
            set(),
        ),
    ]
    for name, body, numstat, expected in cases:
        failures = check_tier(body, numstat)
        text = "\n".join(failures)
        if expected:
            for fragment in expected:
                if fragment not in text:
                    print(
                        f"self-test failed: {name}: missing `{fragment}`",
                        file=sys.stderr,
                    )
                    print(text, file=sys.stderr)
                    return 1
        elif failures:
            print(f"self-test failed: {name}: unexpected failures", file=sys.stderr)
            print(text, file=sys.stderr)
            return 1

    print("check_pr_tier self-test passed")
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

    failures = check_tier(pr_body_from_env(), diff_numstat(args.base, args.head))
    if failures:
        print("PR tier check failed:", file=sys.stderr)
        for failure in failures:
            print(f"- {failure}", file=sys.stderr)
        return 1

    print("PR tier check passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
