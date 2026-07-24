#!/usr/bin/env python3
"""Leak scanner for cross-host benchmark artifacts.

Fails when an artifact references the real host HOME, host session stores,
auth/credential material, or benchmark-private roots. This enforces the
charter isolation rule: target-phase output must not prove access to the
source host's runtime state.

Usage:
  scan_artifacts.py <file-or-dir> [more paths...] [--private-root PATH]...
  scan_artifacts.py --self-test
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

# Temporary sandbox roots where isolated benchmark HOMEs are allowed to live.
ALLOWED_TMP_PREFIXES = ("/tmp/", "/private/tmp/", "/private/var/folders/", "/var/folders/")

LEAK_PATTERNS: list[tuple[str, re.Pattern[str]]] = [
    ("host_home_path", re.compile(r"(?:/Users|/home)/[A-Za-z0-9._-]+/")),
    ("windows_home_path", re.compile(r"[A-Za-z]:\\Users\\[A-Za-z0-9._-]+")),
    (
        "host_session_store",
        re.compile(
            r"\.claude/(?:projects|sessions|statsig|__store)|"
            r"\.claude\.json|"
            r"\.codex/(?:sessions|history\.jsonl|log|rollouts)"
        ),
    ),
    (
        "auth_material",
        re.compile(
            r"\.codex/auth\.json|\.claude/\.credentials|credentials\.json|"
            r"(?:ANTHROPIC|OPENAI)_API_KEY\s*=\s*\S+|"
            r"\bsk-[A-Za-z0-9_-]{16,}|\bghp_[A-Za-z0-9]{20,}|"
            r"Authorization:\s*Bearer\s+\S+"
        ),
    ),
]


def _is_allowed_home_hit(line: str, match: re.Match[str]) -> bool:
    """A home-shaped path inside an allowed tmp root is an isolated bench HOME."""
    start = match.start()
    prefix = line[:start]
    return any(prefix.endswith(root.rstrip("/")) or match.group(0).startswith(root) for root in ALLOWED_TMP_PREFIXES) or any(
        root in line[max(0, start - 40) : start + 1] for root in ALLOWED_TMP_PREFIXES
    )


def scan_text(text: str, private_roots: list[str]) -> list[dict]:
    findings: list[dict] = []
    for lineno, line in enumerate(text.splitlines(), start=1):
        for name, pattern in LEAK_PATTERNS:
            for match in pattern.finditer(line):
                if name == "host_home_path" and _is_allowed_home_hit(line, match):
                    continue
                findings.append(
                    {"pattern": name, "line": lineno, "excerpt": match.group(0)[:120]}
                )
        for root in private_roots:
            if root and root in line:
                findings.append(
                    {"pattern": "benchmark_private_root", "line": lineno, "excerpt": root}
                )
    return findings


def scan_paths(paths: list[Path], private_roots: list[str]) -> dict:
    per_file: dict[str, list[dict]] = {}
    files: list[Path] = []
    for path in paths:
        if path.is_dir():
            files.extend(p for p in sorted(path.rglob("*")) if p.is_file())
        else:
            files.append(path)
    for file_path in files:
        try:
            text = file_path.read_text(encoding="utf-8", errors="replace")
        except OSError as exc:
            per_file[str(file_path)] = [
                {"pattern": "unreadable_artifact", "line": 0, "excerpt": str(exc)}
            ]
            continue
        findings = scan_text(text, private_roots)
        if findings:
            per_file[str(file_path)] = findings
    return {
        "scanned_files": len(files),
        "leaking_files": len(per_file),
        "passed": not per_file,
        "findings": per_file,
    }


def self_test() -> int:
    private_roots = ["/bench-private/codex-home-1234"]
    cases = [
        ("clean text passes", "resolved: true\nrelative/path/report.json\n", []),
        (
            "real macos home fails",
            "read file /Users/alice/project/notes.md\n",
            ["host_home_path"],
        ),
        (
            "real linux home fails",
            "opened /home/bob/work/repo/main.rs\n",
            ["host_home_path"],
        ),
        (
            "isolated tmp home passes",
            "HOME=/private/tmp/bench-run-7/home/Users/agent/ ok\n",
            [],
        ),
        (
            "claude session store fails",
            "loaded ~/.claude/projects/-x-y/session.jsonl\n",
            ["host_session_store"],
        ),
        (
            "codex session store fails",
            "tail .codex/sessions/rollout-2026.jsonl\n",
            ["host_session_store"],
        ),
        (
            "codex auth fails",
            "copied .codex/auth.json into workdir\n",
            ["auth_material"],
        ),
        (
            "api key env fails",
            "OPENAI_API_KEY=abc123secret\n",
            ["auth_material"],
        ),
        (
            "bearer token fails",
            "Authorization: Bearer eyJhbGciOi\n",
            ["auth_material"],
        ),
        (
            "private bench root fails",
            "wrote /bench-private/codex-home-1234/config.toml\n",
            ["benchmark_private_root"],
        ),
        (
            "windows home fails",
            "C:\\Users\\carol\\project\n",
            ["windows_home_path"],
        ),
    ]
    failures = 0
    for name, text, expected in cases:
        got = sorted({f["pattern"] for f in scan_text(text, private_roots)})
        ok = got == sorted(set(expected))
        print(f"{'PASS' if ok else 'FAIL'}: {name} (expected {sorted(set(expected))}, got {got})")
        if not ok:
            failures += 1
    print(f"self-test: {failures} failures")
    return 1 if failures else 0


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("paths", nargs="*", help="artifact files or directories to scan")
    parser.add_argument(
        "--private-root",
        action="append",
        default=[],
        help="benchmark-private path that must not appear in artifacts",
    )
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--json", action="store_true", help="print full JSON report")
    args = parser.parse_args(argv[1:])

    if args.self_test:
        return self_test()
    if not args.paths:
        parser.error("provide artifact paths or --self-test")

    report = scan_paths([Path(p) for p in args.paths], args.private_root)
    if args.json:
        print(json.dumps(report, indent=2))
    else:
        for file_path, findings in report["findings"].items():
            for finding in findings:
                print(
                    f"{file_path}:{finding['line']}: {finding['pattern']}: {finding['excerpt']}"
                )
        print(
            f"scanned {report['scanned_files']} files, "
            f"{report['leaking_files']} leaking: "
            f"{'PASS' if report['passed'] else 'FAIL'}"
        )
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv))
