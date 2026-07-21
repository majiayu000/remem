#!/usr/bin/env python3
"""Validate declared PR tier claims against the actual diff.

PR tiers (`Tier: fastlane|standard|heavy` in the PR body) let small changes
skip the separate-spec-PR process. The tier is only trustworthy if a
machine, not the PR author, verifies the claim. PRs without a `Tier:` line
remain outside tier classification, but every PR must declare
`enforcement_sensitive: true|false` and match the sensitive registry.
"""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
from pathlib import Path

from check_spec_lifecycle import pr_body_from_env

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "checks"))

from sensitive_enforcement import (  # noqa: E402
    classify_sensitive_changes,
    sensitive_registry,
)
from specrail_lib import PackConfig, SpecRailError, load_pack, parse_yaml_subset  # noqa: E402

TIER_RE = re.compile(r"^\s*Tier:\s*(\S+)\s*$", re.I | re.M)
SENSITIVE_RE = re.compile(
    r"^\s*enforcement_sensitive\s*:\s*(true|false)\s*$", re.I | re.M
)
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


def declared_sensitive(body: str) -> tuple[bool | None, list[str]]:
    values = [match.lower() == "true" for match in SENSITIVE_RE.findall(body)]
    if not values:
        return None, ["PR body must declare `enforcement_sensitive: true|false` exactly once."]
    if len(values) != 1:
        return None, ["PR body contains multiple enforcement_sensitive declarations."]
    return values[0], []


def pack_at_revision(revision: str, head_config: PackConfig) -> PackConfig:
    completed = subprocess.run(
        ["git", "show", f"{revision}:workflow.yaml"],
        check=False,
        capture_output=True,
        text=True,
    )
    if completed.returncode != 0:
        detail = completed.stderr.strip() or "git show failed"
        raise SpecRailError(f"cannot load trusted base workflow.yaml: {detail}")
    workflow = parse_yaml_subset(completed.stdout)
    if not isinstance(workflow, dict):
        raise SpecRailError("trusted base workflow.yaml must be a mapping")
    return PackConfig(
        repo=ROOT,
        workflow=workflow,
        states=head_config.states,
        labels=head_config.labels,
    )


def check_sensitive(
    body: str,
    numstat: list[tuple[str, str, str]],
    *,
    head_config: PackConfig | None = None,
    base_config: PackConfig | None = None,
) -> list[str]:
    declaration, failures = declared_sensitive(body)
    if failures:
        return failures
    paths = [path for _added, _deleted, path in numstat]
    try:
        head_config = head_config or load_pack(ROOT)
        base_config = base_config or head_config
        head_registry = sensitive_registry(head_config)
        base_registry = sensitive_registry(base_config)
        removed = {
            key: sorted(set(base_registry[key]) - set(head_registry[key]))
            for key in ("paths", "specs")
        }
        if any(removed.values()):
            details = [
                f"{key}: {', '.join(values)}"
                for key, values in removed.items()
                if values
            ]
            return [
                "Sensitive registry must not remove trusted base entries: "
                + "; ".join(details)
            ]
        classifications = [
            classify_sensitive_changes(
                config,
                ROOT,
                paths,
                paths,
                source="github_changed_files",
            )
            for config in (base_config, head_config)
        ]
    except SpecRailError as exc:
        return [f"Sensitive classification failed closed: {exc}"]
    computed = any(item["enforcement_sensitive"] for item in classifications)
    if computed and declaration is not True:
        matched = sorted(
            {
                path
                for item in classifications
                for path in item["matched_paths"] + item["matched_specs"]
            }
        )
        return [
            "Sensitive registry matched but PR declares false: " + ", ".join(matched)
        ]
    return []


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


def check_pr(
    body: str,
    numstat: list[tuple[str, str, str]],
    *,
    head_config: PackConfig | None = None,
    base_config: PackConfig | None = None,
) -> list[str]:
    return check_tier(body, numstat) + check_sensitive(
        body, numstat, head_config=head_config, base_config=base_config
    )


def self_test() -> int:
    cases = [
        (
            "no tier line passes",
            "Closes #1\nenforcement_sensitive: false",
            [("100", "50", "src/big.rs")],
            set(),
        ),
        (
            "unknown tier fails",
            "Tier: turbo\nenforcement_sensitive: false",
            [("1", "1", "src/lib.rs")],
            {"Unknown tier"},
        ),
        (
            "fastlane small change passes",
            "Tier: fastlane\nCloses #1\nenforcement_sensitive: false",
            [("20", "10", "src/context.rs"), ("300", "0", "docs/notes.md")],
            set(),
        ),
        (
            "fastlane over budget fails",
            "Tier: fastlane\nenforcement_sensitive: false",
            [("40", "20", "src/context.rs")],
            {"limited to 50"},
        ),
        (
            "fastlane protected api path fails",
            "Tier: fastlane\nenforcement_sensitive: false",
            [("5", "5", "src/api/handlers.rs")],
            {"protected path"},
        ),
        (
            "fastlane workflow path fails",
            "Tier: fastlane\nenforcement_sensitive: true",
            [("2", "2", ".github/workflows/ci.yml")],
            {"protected path"},
        ),
        (
            "fastlane schema name fails",
            "Tier: fastlane\nenforcement_sensitive: false",
            [("3", "3", "src/store/schema_v70.rs")],
            {"protected path"},
        ),
        (
            "fastlane binary fails",
            "Tier: fastlane\nenforcement_sensitive: false",
            [("-", "-", "assets/logo.png")],
            {"binary file"},
        ),
        (
            "standard tier passes untouched",
            "Tier: standard\nenforcement_sensitive: true",
            [("500", "200", "src/api/handlers.rs")],
            set(),
        ),
        (
            "case-insensitive tier line",
            "tier: FASTLANE\nenforcement_sensitive: false",
            [("10", "0", "src/lib.rs")],
            set(),
        ),
        (
            "missing sensitive declaration fails",
            "Tier: standard",
            [("1", "0", "src/lib.rs")],
            {"must declare"},
        ),
        (
            "sensitive registry conflict fails",
            "Tier: standard\nenforcement_sensitive: false",
            [("1", "0", "src/rules/compiler.rs")],
            {"Sensitive registry matched"},
        ),
        (
            "sensitive registry match passes with true",
            "Tier: standard\nenforcement_sensitive: true",
            [("1", "0", "src/rules/compiler.rs")],
            set(),
        ),
    ]
    for name, body, numstat, expected in cases:
        failures = check_pr(body, numstat)
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

    head_config = load_pack(ROOT)
    weakened_workflow = dict(head_config.workflow)
    weakened_workflow["enforcement"] = {
        "sensitive_registry": {"paths": [], "specs": []}
    }
    weakened = PackConfig(
        repo=ROOT,
        workflow=weakened_workflow,
        states=head_config.states,
        labels=head_config.labels,
    )
    failures = check_sensitive(
        "enforcement_sensitive: true",
        [("1", "0", "workflow.yaml")],
        head_config=weakened,
        base_config=head_config,
    )
    if not any("must not remove trusted base entries" in item for item in failures):
        print("self-test failed: registry shrinkage did not fail closed", file=sys.stderr)
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

    try:
        head_config = load_pack(ROOT)
        base_config = pack_at_revision(args.base, head_config)
    except SpecRailError as exc:
        print(f"PR tier check failed closed: {exc}", file=sys.stderr)
        return 1
    failures = check_pr(
        pr_body_from_env(),
        diff_numstat(args.base, args.head),
        head_config=head_config,
        base_config=base_config,
    )
    if failures:
        print("PR tier check failed:", file=sys.stderr)
        for failure in failures:
            print(f"- {failure}", file=sys.stderr)
        return 1

    print("PR tier check passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
