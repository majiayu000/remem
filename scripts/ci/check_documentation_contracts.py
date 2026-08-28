#!/usr/bin/env python3
"""Check stable command and navigation contracts in current documentation."""

from __future__ import annotations

import re
import sys
import unicodedata
from pathlib import Path
from urllib.parse import unquote


ROOT = Path(__file__).resolve().parents[2]
README_PATHS = (Path("README.md"), Path("README.zh-CN.md"))
HOMEBREW_DOCS = (*README_PATHS, Path("docs/installation.md"))
CANONICAL_BREW_INSTALL = '"$(brew --prefix remem)/bin/remem" install --target codex'
LINK_PATTERN = re.compile(r"(?<!!)\[[^\]]+\]\(([^)]+)\)")
HEADING_PATTERN = re.compile(r"^#{1,6}\s+(.+?)\s*#*\s*$", re.MULTILINE)


def read(root: Path, relative: Path) -> str:
    return (root / relative).read_text(encoding="utf-8")


def github_slug(heading: str) -> str:
    """Return the stable subset of GitHub's Markdown heading slug algorithm."""
    normalized = unicodedata.normalize("NFKC", heading).strip().lower()
    normalized = re.sub(r"<[^>]+>", "", normalized)
    normalized = normalized.replace("`", "")
    normalized = re.sub(r"[^\w\s-]", "", normalized, flags=re.UNICODE)
    return re.sub(r"[\s-]+", "-", normalized).strip("-")


def heading_anchors(text: str) -> set[str]:
    counts: dict[str, int] = {}
    anchors: set[str] = set()
    for heading in HEADING_PATTERN.findall(text):
        base = github_slug(heading)
        count = counts.get(base, 0)
        counts[base] = count + 1
        anchors.add(base if count == 0 else f"{base}-{count}")
    return anchors


def section(text: str, heading: str) -> str:
    match = re.search(
        rf"^##\s+{re.escape(heading)}\s*$\n(.*?)(?=^##\s+|\Z)",
        text,
        flags=re.MULTILINE | re.DOTALL,
    )
    return match.group(1) if match else ""


def check_homebrew_commands(root: Path, violations: list[str]) -> None:
    unsafe = re.compile(
        r'REMEM_INSTALL_BINARY=[^\n]*\$\(brew --prefix remem\)[^\n]*\bremem install'
    )
    for path in HOMEBREW_DOCS:
        text = read(root, path)
        if unsafe.search(text) or CANONICAL_BREW_INSTALL not in text:
            violations.append(
                f"{path}: Homebrew setup must execute the canonical formula binary directly"
            )


def check_exports(root: Path, violations: list[str]) -> None:
    explicit_current_dir = re.compile(
        r"^\s*remem export\b[^\n]*--project(?:=|\s+)(?:[\"']?\$PWD[\"']?|\.)",
        flags=re.MULTILINE,
    )
    for path in README_PATHS:
        text = read(root, path)
        export_lines = re.findall(r"^\s*remem export\b.*$", text, flags=re.MULTILINE)
        if len(export_lines) < 2:
            violations.append(f"{path}: document both current-project export forms")
        if explicit_current_dir.search(text):
            violations.append(
                f"{path}: omit --project for current-project export so the CLI can canonicalize it"
            )


def check_channel_switch(root: Path, violations: list[str]) -> None:
    path = Path("docs/installation.md")
    upgrade = section(read(root, path), "Upgrade an existing installation")
    required = (
        "/old/path/remem uninstall",
        "brew uninstall remem",
        "npm uninstall -g @remem-ai/remem",
        "cargo uninstall remem-ai",
        "rm /exact/path/to/old/remem",
        "/new/path/remem install --target codex",
    )
    missing = [command for command in required if command not in upgrade]
    if missing:
        violations.append(
            f"{path}: channel switch must remove old host entries and executable; "
            f"missing {', '.join(missing)}"
        )


def check_hub_links(root: Path, violations: list[str]) -> None:
    source = root / "docs/README.md"
    for raw_target in LINK_PATTERN.findall(source.read_text(encoding="utf-8")):
        target = raw_target.strip().split(maxsplit=1)[0].strip("<>")
        if target.startswith(("http://", "https://", "mailto:")):
            continue
        path_text, separator, fragment = target.partition("#")
        destination = source if not path_text else (source.parent / unquote(path_text)).resolve()
        if not destination.exists():
            violations.append(f"docs/README.md: missing local Markdown target {target}")
            continue
        if not separator or not fragment or destination.suffix.lower() != ".md":
            continue
        anchors = heading_anchors(destination.read_text(encoding="utf-8"))
        decoded_fragment = unquote(fragment).lower()
        if decoded_fragment not in anchors:
            violations.append(
                f"docs/README.md: missing Markdown anchor {decoded_fragment} in "
                f"{destination.relative_to(root.resolve())}"
            )


def check_sessionstart_smoke(root: Path, violations: list[str]) -> None:
    path = Path("docs/README.md")
    text = read(root, path)
    smoke = section(text, "SessionStart context smoke")
    commands = [
        line
        for line in smoke.splitlines()
        if re.search(r"(?:\bremem|\bcargo run\b.*\s--)\s+context\b", line)
    ]
    initialization = re.search(
        r'REMEM_DATA_DIR=["\']?\$tmpdir["\']?[^\n]*(?:\bremem|\bcargo run\b.*\s--)\s+encrypt\b',
        smoke,
    )
    valid_commands = [line for line in commands if "REMEM_DATA_DIR=" in line]
    first_context = smoke.find(commands[0]) if commands else -1
    if (
        len(commands) != 2
        or len(valid_commands) != 2
        or initialization is None
        or initialization.start() > first_context
    ):
        violations.append(
            f"{path}: isolated SessionStart smoke must initialize the encrypted store "
            "before both context commands"
        )


def check_fts_semantics(root: Path, violations: list[str]) -> None:
    path = Path("docs/memory-lifecycle.md")
    text = " ".join(read(root, path).lower().split())
    required = (
        "memories_fts",
        "`active`, `stale`, and `archived`",
        "query time",
        "include_stale=true",
    )
    if any(marker not in text for marker in required):
        violations.append(
            f"{path}: document the all-status FTS index and query-time lifecycle filtering"
        )


def check(root: Path = ROOT) -> list[str]:
    violations: list[str] = []
    check_homebrew_commands(root, violations)
    check_exports(root, violations)
    check_channel_switch(root, violations)
    check_hub_links(root, violations)
    check_sessionstart_smoke(root, violations)
    check_fts_semantics(root, violations)
    return violations


def main() -> int:
    violations = check(ROOT)
    if violations:
        print("documentation contract check failed:", file=sys.stderr)
        for violation in violations:
            print(f"- {violation}", file=sys.stderr)
        return 1
    print("documentation contracts verified")
    return 0


if __name__ == "__main__":
    sys.exit(main())
