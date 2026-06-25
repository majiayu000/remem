#!/usr/bin/env python3
"""Check public release and discoverability surfaces that CI can prove locally."""

from __future__ import annotations

import re
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]

README_BADGES = [
    "actions/workflows/ci.yml/badge.svg",
    "img.shields.io/github/v/release/majiayu000/remem",
    "img.shields.io/crates/v/remem-ai",
    "img.shields.io/npm/v/%40remem-ai%2Fremem",
    "License-MIT",
]

README_REQUIRED_TEXT = [
    "brew install majiayu000/tap/remem",
    "npm install -g @remem-ai/remem",
    "cargo install remem-ai --bin remem",
    "remem doctor",
    "remem search \"last decision\"",
    "GitHub Releases: prebuilt binaries",
]

ROOT_REQUIRED_FILES = [
    "README.md",
    "README.zh-CN.md",
    "CHANGELOG.md",
    "CODE_OF_CONDUCT.md",
    "CONTRIBUTING.md",
    "SECURITY.md",
    "LICENSE",
    "assets/remem-demo.gif",
    "assets/social-preview.svg",
    "docs/release-lifecycle.md",
    "docs/maintenance/file-size-debt.md",
]

SITE_PAGES = [
    "site/index.html",
    "site/claude-code-memory/index.html",
    "site/codex-memory/index.html",
    "site/mcp-memory-server/index.html",
    "site/compare/built-in-memory/index.html",
]


def fail(message: str) -> None:
    print(f"FAIL: {message}", file=sys.stderr)
    sys.exit(1)


def require_file(path: str) -> None:
    if not (ROOT / path).is_file():
        fail(f"missing {path}")


def require_contains(label: str, text: str, needle: str) -> None:
    if needle not in text:
        fail(f"{label} is missing {needle!r}")


def require_site_page(path: str) -> None:
    full = ROOT / path
    require_file(path)
    text = full.read_text(encoding="utf-8")
    for needle in [
        "<title>",
        'name="description"',
        'rel="canonical"',
        'property="og:title"',
        'name="twitter:card"',
        'name="robots" content="index,follow"',
    ]:
        require_contains(path, text, needle)
    if len(re.findall(r"<h1\b", text)) != 1:
        fail(f"{path} must contain exactly one h1")


def main() -> int:
    for path in ROOT_REQUIRED_FILES:
        require_file(path)

    readme = (ROOT / "README.md").read_text(encoding="utf-8")
    for needle in README_BADGES + README_REQUIRED_TEXT:
        require_contains("README.md", readme, needle)

    zh_readme = (ROOT / "README.zh-CN.md").read_text(encoding="utf-8")
    for needle in README_BADGES:
        require_contains("README.zh-CN.md", zh_readme, needle)

    robots = (ROOT / "site/robots.txt").read_text(encoding="utf-8")
    require_contains("site/robots.txt", robots, "Sitemap: https://majiayu000.github.io/remem/sitemap.xml")
    sitemap = (ROOT / "site/sitemap.xml").read_text(encoding="utf-8")
    for url in [
        "https://majiayu000.github.io/remem/",
        "https://majiayu000.github.io/remem/codex-memory/",
        "https://majiayu000.github.io/remem/claude-code-memory/",
        "https://majiayu000.github.io/remem/mcp-memory-server/",
    ]:
        require_contains("site/sitemap.xml", sitemap, url)

    for page in SITE_PAGES:
        require_site_page(page)

    codex_page = (ROOT / "site/codex-memory/index.html").read_text(encoding="utf-8")
    require_contains("site/codex-memory/index.html", codex_page, "application/ld+json")

    print("public surface check: ok")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
