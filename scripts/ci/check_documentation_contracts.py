#!/usr/bin/env python3
"""Check stable command and navigation contracts in current documentation."""

from __future__ import annotations

import re
import shlex
import sys
import unicodedata
from dataclasses import dataclass
from pathlib import Path
from urllib.parse import unquote


ROOT = Path(__file__).resolve().parents[2]
README_PATHS = (Path("README.md"), Path("README.zh-CN.md"))
HOMEBREW_DOCS = (*README_PATHS, Path("docs/installation.md"))
CURRENT_EXPORT_DOCS = (*README_PATHS, Path("docs/specs/project-memory-pack/PRODUCT.md"))
SESSIONSTART_SMOKE_SCRIPT = Path("scripts/ci/smoke_sessionstart_context_gate.sh")
SESSIONSTART_SMOKE_GUIDE = Path("docs/sessionstart-context-smoke.md")
SESSIONSTART_SMOKE_ROUTES = {
    Path("README.md"): "scripts/ci/smoke_sessionstart_context_gate.sh",
    Path("README.zh-CN.md"): "scripts/ci/smoke_sessionstart_context_gate.sh",
    Path("docs/README.md"): "../scripts/ci/smoke_sessionstart_context_gate.sh",
    SESSIONSTART_SMOKE_GUIDE: "scripts/ci/smoke_sessionstart_context_gate.sh",
}
LINK_PATTERN = re.compile(r"(?<!!)\[[^\]]+\]\(([^)]+)\)")
HEADING_PATTERN = re.compile(r"^#{1,6}\s+(.+?)\s*#*\s*$", re.MULTILINE)
SHELL_FENCE_PATTERN = re.compile(
    r"^[ \t]*```(?:bash|sh|shell)\s*\n(.*?)^[ \t]*```\s*$",
    re.MULTILINE | re.DOTALL,
)


@dataclass(frozen=True)
class ShellCommand:
    raw: str
    assignments: dict[str, str]
    argv: tuple[str, ...]


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


def contract_region(text: str, contract: str) -> str | None:
    start = f"<!-- remem-doc-contract:{contract}:start -->"
    end = f"<!-- remem-doc-contract:{contract}:end -->"
    if text.count(start) != 1 or text.count(end) != 1:
        return None
    before, region_and_end = text.split(start, maxsplit=1)
    del before
    region, trailing = region_and_end.split(end, maxsplit=1)
    del trailing
    return region


def shell_tokens(line: str) -> list[str]:
    lexer = shlex.shlex(line, posix=True, punctuation_chars="|")
    lexer.whitespace_split = True
    lexer.commenters = "#"
    return list(lexer)


def shell_commands(text: str) -> list[ShellCommand]:
    commands: list[ShellCommand] = []
    for fenced in SHELL_FENCE_PATTERN.findall(text):
        logical = re.sub(r"\\\s*\n", " ", fenced)
        for raw_line in logical.splitlines():
            raw_line = raw_line.strip()
            if not raw_line or raw_line.startswith("#"):
                continue
            tokens = shell_tokens(raw_line)
            segment: list[str] = []
            for token in [*tokens, "|"]:
                if token == "|":
                    if segment:
                        assignments: dict[str, str] = {}
                        while segment and re.fullmatch(
                            r"[A-Za-z_][A-Za-z0-9_]*=.*", segment[0]
                        ):
                            name, value = segment.pop(0).split("=", maxsplit=1)
                            assignments[name] = value
                        commands.append(
                            ShellCommand(raw_line, assignments, tuple(segment))
                        )
                    segment = []
                else:
                    segment.append(token)
    return commands


def invokes_remem(command: ShellCommand, subcommand: str) -> bool:
    if not command.argv:
        return False
    executable = command.argv[0]
    if executable == "remem" or executable.endswith("/bin/remem"):
        return len(command.argv) > 1 and command.argv[1] == subcommand
    if executable == "cargo" and "--" in command.argv:
        separator = command.argv.index("--")
        return len(command.argv) > separator + 1 and command.argv[separator + 1] == subcommand
    return False


def has_option(command: ShellCommand, option: str) -> bool:
    return any(arg == option or arg.startswith(f"{option}=") for arg in command.argv)


def check_homebrew_commands(root: Path, violations: list[str]) -> None:
    for path in HOMEBREW_DOCS:
        installs = [
            command
            for command in shell_commands(read(root, path))
            if invokes_remem(command, "install")
        ]
        canonical = [
            command
            for command in installs
            if command.argv[0] == "$(brew --prefix remem)/bin/remem"
            and command.assignments.get("REMEM_INSTALL_BINARY") is None
        ]
        if not canonical:
            violations.append(
                f"{path}: Homebrew setup must execute the canonical formula binary directly"
            )


def check_exports(root: Path, violations: list[str]) -> None:
    for path in CURRENT_EXPORT_DOCS:
        region = contract_region(read(root, path), "current-project-export")
        if region is None:
            violations.append(f"{path}: missing current-project export contract block")
            continue
        exports = [
            command for command in shell_commands(region) if invokes_remem(command, "export")
        ]
        if not exports:
            violations.append(f"{path}: current-project export contract has no export command")
            continue
        if any(has_option(command, "--project") for command in exports):
            violations.append(
                f"{path}: omit --project for current-project export so the CLI can canonicalize it"
            )
        if path in README_PATHS and not (
            any(has_option(command, "--markdown") for command in exports)
            and any(has_option(command, "--pack") for command in exports)
        ):
            violations.append(f"{path}: document both current-project export forms")


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
    script = root / SESSIONSTART_SMOKE_SCRIPT
    if not script.is_file() or script.stat().st_mode & 0o111 == 0:
        violations.append(
            f"{SESSIONSTART_SMOKE_SCRIPT}: SessionStart smoke entry must exist and be executable"
        )

    for path, route in SESSIONSTART_SMOKE_ROUTES.items():
        text = read(root, path)
        if route not in text:
            violations.append(f"{path}: route SessionStart smoke to {route}")

    guide = read(root, SESSIONSTART_SMOKE_GUIDE)
    region = contract_region(guide, "isolated-sessionstart-smoke")
    expected_region = (
        "\n```bash\n"
        f"{SESSIONSTART_SMOKE_SCRIPT.as_posix()}\n"
        "```\n"
    )
    if region != expected_region:
        violations.append(
            f"{SESSIONSTART_SMOKE_GUIDE}: SessionStart smoke contract must only invoke "
            f"{SESSIONSTART_SMOKE_SCRIPT}"
        )

    direct_context = re.compile(
        r"(?:\bremem[ \t]+context(?:[ \t]|$)|"
        r"\bcargo[ \t]+run\b[^\n]*[ \t]--[ \t]+context(?:[ \t]|$))",
        flags=re.MULTILINE,
    )
    copied_markers = ("gate-smoke", "mktemp -d", "wc -c")
    for path in SESSIONSTART_SMOKE_ROUTES:
        text = read(root, path)
        if direct_context.search(text) or any(marker in text for marker in copied_markers):
            violations.append(
                f"{path}: do not copy SessionStart smoke implementation; invoke "
                f"{SESSIONSTART_SMOKE_SCRIPT}"
            )


def check_fts_semantics(root: Path, violations: list[str]) -> None:
    path = Path("docs/memory-lifecycle.md")
    region = contract_region(read(root, path), "memories-fts-lifecycle")
    fields: dict[str, str] = {}
    if region is not None:
        for line in region.splitlines():
            cells = [cell.strip().strip("`") for cell in line.strip().strip("|").split("|")]
            if len(cells) == 2 and cells[0] not in {"Invariant", "---"}:
                fields[cells[0]] = cells[1]
    if fields != {
        "Indexed statuses": "active, stale, archived",
        "Lifecycle visibility": "post-JOIN query-time filter",
    }:
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
