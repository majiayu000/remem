#!/usr/bin/env python3
"""Prevent new oversized source files while existing split debt is retired."""

from __future__ import annotations

import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
MAX_LINES = 800
SOURCE_SUFFIXES = {".rs", ".js", ".py", ".sh"}
SKIP_PARTS = {".git", "target", "node_modules"}

# Baseline captured on 2026-06-25. These files must not grow further and should
# be split down over time; see docs/maintenance/file-size-debt.md.
ALLOWLIST = {
    "plugins/remem/apps/remem/server.test.js": 803,
    "src/api/tests/web_regressions.rs": 884,
    "src/api/tests.rs": 2032,
    "src/cli/tests.rs": 922,
    "src/context/tests/load.rs": 823,
    "src/db/extraction/tests.rs": 896,
    "src/db/query/stats/tests.rs": 822,
    "src/doctor/tests.rs": 858,
    "src/git_trace.rs": 833,
    "src/graph_candidate/tests.rs": 1221,
    "src/mcp/server/tests.rs": 974,
    "src/memory/current_state/tests.rs": 1193,
    "src/memory/staleness/tests.rs": 803,
    "src/migrate/tests.rs": 838,
    "src/retrieval/search/memory/tests.rs": 809,
    "tests/benchmark.rs": 1181,
}


def line_count(path: Path) -> int:
    with path.open("r", encoding="utf-8", errors="replace") as handle:
        return sum(1 for _ in handle)


def main() -> int:
    errors: list[str] = []
    seen_allowlist: set[str] = set()
    for path in sorted(ROOT.rglob("*")):
        if not path.is_file() or path.suffix not in SOURCE_SUFFIXES:
            continue
        if any(part in SKIP_PARTS for part in path.parts):
            continue
        rel = path.relative_to(ROOT).as_posix()
        lines = line_count(path)
        if rel in ALLOWLIST:
            seen_allowlist.add(rel)
            if lines > ALLOWLIST[rel]:
                errors.append(f"{rel}: {lines} lines, allowlisted baseline is {ALLOWLIST[rel]}")
            continue
        if lines > MAX_LINES:
            errors.append(f"{rel}: {lines} lines exceeds {MAX_LINES}; split before merging")

    missing = sorted(set(ALLOWLIST) - seen_allowlist)
    if missing:
        errors.append("allowlisted oversized files disappeared; update check_file_size.py: " + ", ".join(missing))

    if errors:
        print("file size check failed:", file=sys.stderr)
        for error in errors:
            print(f"  - {error}", file=sys.stderr)
        print("See docs/maintenance/file-size-debt.md.", file=sys.stderr)
        return 1

    print("file size check: ok")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
