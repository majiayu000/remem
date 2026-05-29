#!/usr/bin/env python3
"""Require Cargo.toml version bumps for binary-impacting pull requests."""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
import tomllib


TRIGGER_FILES = {"Cargo.lock"}
TRIGGER_PREFIXES = ("src/", "migrations/")
SEMVER_RE = re.compile(
    r"^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)"
    r"(?:-([0-9A-Za-z.-]+))?(?:\+[0-9A-Za-z.-]+)?$"
)


def git(*args: str) -> str:
    completed = subprocess.run(
        ["git", *args],
        check=True,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    return completed.stdout


def cargo_toml_at(ref: str) -> str:
    if ref == "WORKTREE":
        with open("Cargo.toml", "rb") as handle:
            return handle.read().decode("utf-8")
    return git("show", f"{ref}:Cargo.toml")


def package_version(ref: str) -> str:
    document = tomllib.loads(cargo_toml_at(ref))
    version = document.get("package", {}).get("version")
    if not isinstance(version, str) or not version.strip():
        raise ValueError(f"{ref}: Cargo.toml is missing package.version")
    return version


def changed_files(base_ref: str, head_ref: str) -> list[str]:
    if head_ref == "WORKTREE":
        output = git("diff", "--name-only", base_ref, "--")
    else:
        output = git("diff", "--name-only", f"{base_ref}..{head_ref}", "--")
    return [line.strip() for line in output.splitlines() if line.strip()]


def requires_bump(path: str) -> bool:
    return path in TRIGGER_FILES or path.startswith(TRIGGER_PREFIXES)


def parse_semver(version: str) -> tuple[tuple[int, int, int], list[str] | None]:
    match = SEMVER_RE.match(version)
    if not match:
        raise ValueError(f"unsupported semver package.version: {version!r}")
    core = tuple(int(match.group(index)) for index in range(1, 4))
    prerelease = match.group(4)
    return core, prerelease.split(".") if prerelease else None


def compare_identifier(left: str, right: str) -> int:
    left_numeric = left.isdigit()
    right_numeric = right.isdigit()
    if left_numeric and right_numeric:
        return (int(left) > int(right)) - (int(left) < int(right))
    if left_numeric != right_numeric:
        return -1 if left_numeric else 1
    return (left > right) - (left < right)


def compare_semver(left: str, right: str) -> int:
    left_core, left_pre = parse_semver(left)
    right_core, right_pre = parse_semver(right)
    if left_core != right_core:
        return (left_core > right_core) - (left_core < right_core)
    if left_pre is None and right_pre is None:
        return 0
    if left_pre is None:
        return 1
    if right_pre is None:
        return -1
    for left_part, right_part in zip(left_pre, right_pre):
        compared = compare_identifier(left_part, right_part)
        if compared:
            return compared
    return (len(left_pre) > len(right_pre)) - (len(left_pre) < len(right_pre))


def format_paths(paths: list[str]) -> str:
    visible = paths[:20]
    rendered = "\n".join(f"  - {path}" for path in visible)
    remaining = len(paths) - len(visible)
    if remaining > 0:
        rendered += f"\n  - ... and {remaining} more"
    return rendered


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Fail when binary-impacting changes do not bump Cargo.toml version."
    )
    parser.add_argument("base_ref", help="Base git ref to compare against, e.g. origin/main")
    parser.add_argument("head_ref", nargs="?", default="HEAD", help="Head git ref")
    args = parser.parse_args()

    files = changed_files(args.base_ref, args.head_ref)
    triggering = [path for path in files if requires_bump(path)]
    if not triggering:
        print("version bump check: no binary-impacting files changed")
        return 0

    base_version = package_version(args.base_ref)
    head_version = package_version(args.head_ref)
    if compare_semver(head_version, base_version) <= 0:
        print(
            "version bump check failed: binary-impacting files changed without a "
            "Cargo.toml package.version increase",
            file=sys.stderr,
        )
        print(f"base version: {base_version}", file=sys.stderr)
        print(f"head version: {head_version}", file=sys.stderr)
        print("triggering files:", file=sys.stderr)
        print(format_paths(triggering), file=sys.stderr)
        return 1

    print(
        "version bump check: "
        f"{head_version} > {base_version} for {len(triggering)} binary-impacting file(s)"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
