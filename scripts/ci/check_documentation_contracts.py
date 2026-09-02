#!/usr/bin/env python3
"""Check stable commands, bilingual facts, and local links in current docs."""

from __future__ import annotations

import re
import shlex
import sys
from dataclasses import dataclass
from html.parser import HTMLParser
from pathlib import Path
from urllib.parse import unquote, urlsplit

try:
    from markdown_it import MarkdownIt
except ModuleNotFoundError as exc:
    raise SystemExit(
        "missing CI dependency markdown-it-py; run "
        "python3 -m pip install --requirement scripts/ci/requirements.txt"
    ) from exc


ROOT = Path(__file__).resolve().parents[2]
README_PATHS = (Path("README.md"), Path("README.zh-CN.md"))
LOCAL_LINK_SOURCES = (*README_PATHS, Path("docs/README.md"))
MARKDOWN_SUFFIXES = {".md", ".markdown"}
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
SHELL_FENCE_PATTERN = re.compile(
    r"^[ \t]*```(?:bash|sh|shell)\s*\n(.*?)^[ \t]*```\s*$",
    re.MULTILINE | re.DOTALL,
)
MARKDOWN = MarkdownIt("commonmark", {"html": True}).enable(
    ["table", "strikethrough"]
)


@dataclass(frozen=True)
class ShellCommand:
    raw: str
    assignments: dict[str, str]
    argv: tuple[str, ...]


@dataclass(frozen=True)
class BilingualInvariant:
    name: str
    tokens: tuple[str, ...]
    affirmative_clauses: tuple[str, str] | None = None


class LinkAttributeParser(HTMLParser):
    def __init__(self) -> None:
        super().__init__(convert_charrefs=True)
        self.targets: list[str] = []

    def handle_starttag(
        self, tag: str, attrs: list[tuple[str, str | None]]
    ) -> None:
        del tag
        self.targets.extend(
            value
            for name, value in attrs
            if name.lower() in {"href", "src"} and value
        )


BILINGUAL_README_INVARIANTS = (
    BilingualInvariant(
        "supported install channels",
        (
            "brew install majiayu000/tap/remem",
            "curl -fsSL https://raw.githubusercontent.com/majiayu000/remem/main/install.sh",
            "npm install -g @remem-ai/remem",
            "cargo install remem-ai --bin remem",
        ),
    ),
    BilingualInvariant(
        "first-run verification commands",
        ('remem doctor', 'remem status', 'remem search "last decision"'),
    ),
    BilingualInvariant("documentation jump page", ("docs/README.md",)),
    BilingualInvariant("security policy", ("SECURITY.md",)),
    BilingualInvariant("current API contract", ("docs/specs/SPEC-web-api.md",)),
    BilingualInvariant("current spec index", ("docs/specs/README.md",)),
    BilingualInvariant("changelog", ("CHANGELOG.md",)),
    BilingualInvariant("contribution guide", ("CONTRIBUTING.md",)),
    BilingualInvariant(
        "Cursor v1 limitation",
        ("remem install --target cursor",),
        (
            "not install automatic capture hooks",
            "不会安装自动捕获 hook",
        ),
    ),
    BilingualInvariant(
        "localhost bearer-token API",
        ("127.0.0.1", "Authorization: Bearer"),
        (
            "The REST API binds to `127.0.0.1` and requires a bearer token.",
            "REST API 只绑定 `127.0.0.1`，并要求 bearer token。",
        ),
    ),
    BilingualInvariant(
        "safe uninstall and data retention",
        ("remem uninstall --dry-run", "REMEM_DATA_DIR"),
        (
            "The encrypted database remains in the configured `REMEM_DATA_DIR`.",
            "加密数据库会保留在配置的 `REMEM_DATA_DIR`。",
        ),
    ),
    BilingualInvariant(
        "public benchmark claim boundary",
        ("directional_only_no_public_claim",),
        (
            "does not support public benchmark claims",
            "不能用于对外 benchmark 声明",
        ),
    ),
    BilingualInvariant("shared demo asset", ("assets/remem-recall-demo.gif",)),
)


def read(root: Path, relative: Path) -> str:
    return (root / relative).read_text(encoding="utf-8")


def slug_from_visible_text(visible: str) -> str:
    slug: list[str] = []
    for character in visible.strip().lower():
        if character == " ":
            slug.append("-")
        elif character in {"-", "_"} or character.isalnum():
            slug.append(character)
        elif character.isspace():
            continue
    return "".join(slug)


def inline_visible_text(token) -> str:
    visible: list[str] = []
    for child in token.children or ():
        if child.type in {"text", "code_inline", "image"}:
            visible.append(child.content)
        elif child.type in {"softbreak", "hardbreak"}:
            visible.append(" ")
    return "".join(visible)


def github_slug(heading: str) -> str:
    """Return GitHub's heading anchor form from rendered inline text."""
    inline = MARKDOWN.parseInline(heading)[0]
    return slug_from_visible_text(inline_visible_text(inline))


def heading_anchors(text: str) -> set[str]:
    anchors: set[str] = set()
    tokens = MARKDOWN.parse(text)
    for index, token in enumerate(tokens[:-1]):
        if token.type != "heading_open" or tokens[index + 1].type != "inline":
            continue
        base = slug_from_visible_text(inline_visible_text(tokens[index + 1]))
        candidate = base
        suffix = 1
        while candidate in anchors:
            candidate = f"{base}-{suffix}"
            suffix += 1
        anchors.add(candidate)
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


def markdown_destinations(text: str):
    for token in MARKDOWN.parse(text):
        line_number = token.map[0] + 1 if token.map is not None else 1
        if token.type == "html_block":
            parser = LinkAttributeParser()
            parser.feed(token.content)
            yield from ((line_number, target) for target in parser.targets)
        if token.type != "inline":
            continue
        for child in token.children or ():
            if child.type == "html_inline":
                parser = LinkAttributeParser()
                parser.feed(child.content)
                yield from ((line_number, target) for target in parser.targets)
                continue
            attribute = "href" if child.type == "link_open" else "src"
            if child.type not in {"link_open", "image"}:
                continue
            target = child.attrGet(attribute)
            if target:
                yield line_number, target


def check_local_markdown_links(root: Path, violations: list[str]) -> None:
    repository = root.resolve()
    anchor_cache: dict[Path, set[str]] = {}
    for relative_source in LOCAL_LINK_SOURCES:
        source = root / relative_source
        for line_number, target in markdown_destinations(read(root, relative_source)):
            parsed = urlsplit(target)
            target_label = unquote(target)
            if parsed.scheme or parsed.netloc or target.startswith(("//", "/")):
                continue
            path_text = unquote(parsed.path)
            destination = source if not path_text else source.parent / path_text
            destination = destination.resolve()
            try:
                destination.relative_to(repository)
            except ValueError:
                violations.append(
                    f"{relative_source}:{line_number}: {target_label}: local Markdown target "
                    "escapes the repository"
                )
                continue
            if not destination.exists():
                violations.append(
                    f"{relative_source}:{line_number}: {target_label}: "
                    "missing local Markdown target"
                )
                continue
            if not parsed.fragment or destination.suffix.lower() not in MARKDOWN_SUFFIXES:
                continue
            anchors = anchor_cache.get(destination)
            if anchors is None:
                anchors = heading_anchors(destination.read_text(encoding="utf-8"))
                anchor_cache[destination] = anchors
            fragment = unquote(parsed.fragment)
            if fragment not in anchors:
                destination_label = destination.relative_to(repository)
                violations.append(
                    f"{relative_source}:{line_number}: {target_label}: missing Markdown anchor "
                    f"{fragment} in {destination_label}"
                )


def check_bilingual_readme_invariants(root: Path, violations: list[str]) -> None:
    for path_index, path in enumerate(README_PATHS):
        text = read(root, path)
        for invariant in BILINGUAL_README_INVARIANTS:
            missing = [token for token in invariant.tokens if token not in text]
            if (
                invariant.affirmative_clauses is not None
                and invariant.affirmative_clauses[path_index] not in text
            ):
                missing.append("affirmative contract clause")
            if missing:
                violations.append(
                    f"{path}: missing bilingual invariant {invariant.name}: "
                    + ", ".join(missing)
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
        "python3 scripts/ci/run_sessionstart_context_gate_smoke.py\n"
        "```\n"
    )
    if region != expected_region:
        violations.append(
            f"{SESSIONSTART_SMOKE_GUIDE}: SessionStart smoke contract must invoke the "
            "artifact-resolving runner"
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
    check_local_markdown_links(root, violations)
    check_bilingual_readme_invariants(root, violations)
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
