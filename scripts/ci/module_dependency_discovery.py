#!/usr/bin/env python3
"""Conservative Rust module-dependency discovery for the GH969 guard."""

from __future__ import annotations

import hashlib
import re
from collections import defaultdict
from dataclasses import dataclass
from pathlib import Path

LAYER_ROOTS: tuple[tuple[str, tuple[str, ...]], ...] = (
    (
        "foundation/domain",
        (
            "atomic_file",
            "build_info",
            "git_util",
            "identity",
            "log",
            "perf",
            "project_alias",
            "project_id",
            "runtime_config",
        ),
    ),
    (
        "storage",
        (
            "captured_git",
            "db",
            "git_evidence",
            "git_trace",
            "migrate",
            "spill_queue",
        ),
    ),
    (
        "memory/retrieval",
        (
            "graph_candidate",
            "memory",
            "memory_candidate",
            "retrieval",
            "rules",
            "truth",
            "user_context",
            "workstream",
        ),
    ),
    (
        "application",
        (
            "ai",
            "context",
            "context_bundle",
            "dream",
            "extraction_worker",
            "ingest",
            "maintenance",
            "observation_extract",
            "retrieval_router",
            "session_activity",
            "session_rollup",
            "summarize",
            "timeline",
            "worker",
        ),
    ),
    (
        "adapters",
        (
            "adapter",
            "api",
            "cli",
            "cursor_hook",
            "hook_cli",
            "hook_integrity",
            "hook_runtime",
            "hook_stdin",
            "install",
            "mcp",
            "observe",
        ),
    ),
    ("evidence/diagnostics", ("eval", "doctor")),
)

ROOT_LAYERS = {
    root: (rank, layer)
    for rank, (layer, roots) in enumerate(LAYER_ROOTS)
    for root in roots
}
IDENTIFIER = r"[A-Za-z_][A-Za-z0-9_]*"
ROOT_MOD_RE = re.compile(
    rf"^[ \t]*(?:#\s*\[[^]]+\]\s*)*(?:pub(?:\s*\([^)]*\))?\s+)?mod\s+({IDENTIFIER})\s*;",
    re.M,
)
USE_START_RE = re.compile(r"\b(?:pub(?:\s*\([^)]*\))?\s+)?use\b")
ABSOLUTE_PATH_RE = re.compile(
    rf"(?<![A-Za-z0-9_])(?P<prefix>::\s*)?(?P<base>crate|remem)\s*::\s*(?P<root>{IDENTIFIER})"
)
RELATIVE_PATH_RE = re.compile(
    rf"(?<![A-Za-z0-9_])(?P<prefix>super(?:\s*::\s*super)*)\s*::\s*(?P<name>{IDENTIFIER})"
)
INLINE_MOD_RE = re.compile(
    rf"\bmod\s+(?P<name>{IDENTIFIER})\s*\{{"
)
TEST_CFG_RE = re.compile(r"#\s*\[\s*cfg\s*\(\s*test\s*\)\s*\]")
EXTERNAL_MOD_RE = re.compile(
    rf"(?P<attrs>(?:#\s*\[[^]]+\]\s*)*)"
    rf"(?:pub(?:\s*\([^)]*\))?\s+)?mod\s+(?P<name>{IDENTIFIER})\s*;",
    re.M,
)
PATH_ATTR_RE = re.compile(r'#\s*\[\s*path\s*=\s*"(?P<path>[^"\n]+)"\s*\]')


class DiscoveryError(RuntimeError):
    """Raised when ownership or conservative parsing cannot be proven."""


def mask_rust_structure(text: str) -> str:
    """Mask Rust comments and literals while preserving offsets and newlines."""
    chars = list(text)

    def erase(start: int, end: int) -> None:
        for offset in range(start, end):
            if chars[offset] != "\n":
                chars[offset] = " "

    index = 0
    while index < len(text):
        if text.startswith("//", index):
            end = text.find("\n", index)
            end = len(text) if end < 0 else end
            erase(index, end)
            index = end
            continue
        if text.startswith("/*", index):
            depth = 1
            end = index + 2
            while end < len(text) and depth:
                if text.startswith("/*", end):
                    depth += 1
                    end += 2
                elif text.startswith("*/", end):
                    depth -= 1
                    end += 2
                else:
                    end += 1
            if depth:
                raise DiscoveryError(f"unterminated block comment at byte {index}")
            erase(index, end)
            index = end
            continue

        raw = re.match(r"(?:br|cr|r)(?P<hashes>#{0,255})\"", text[index:])
        if raw:
            terminator = '"' + raw.group("hashes")
            end = text.find(terminator, index + raw.end())
            if end < 0:
                raise DiscoveryError(f"unterminated raw string at byte {index}")
            end += len(terminator)
            erase(index, end)
            index = end
            continue

        prefix = 1 if text.startswith(('b"', 'c"'), index) else 0
        if text[index + prefix : index + prefix + 1] == '"':
            end = index + prefix + 1
            while end < len(text):
                if text[end] == "\\":
                    end += 2
                elif text[end] == '"':
                    end += 1
                    break
                else:
                    end += 1
            else:
                raise DiscoveryError(f"unterminated string at byte {index}")
            erase(index, end)
            index = end
            continue

        char_prefix = 1 if text.startswith("b'", index) else 0
        if text[index + char_prefix : index + char_prefix + 1] == "'":
            end = index + char_prefix + 1
            if end < len(text) and text[end] == "\\":
                end += 2
            else:
                end += 1
            if end < len(text) and text[end] == "'":
                end += 1
                erase(index, end)
                index = end
                continue
        index += 1
    return "".join(chars)


@dataclass(frozen=True)
class Site:
    source: str
    target: str
    path: str
    line: int
    kind: str
    signature: str
    occurrence: int

    def baseline_value(self) -> dict[str, object]:
        return {
            "path": self.path,
            "kind": self.kind,
            "signature": self.signature,
            "occurrence": self.occurrence,
        }

    def key(self) -> tuple[str, str, str, int]:
        return (self.path, self.kind, self.signature, self.occurrence)


@dataclass(frozen=True)
class ScanResult:
    root_layers: dict[str, tuple[int, str]]
    sites: tuple[Site, ...]
    cyclic_components: tuple[tuple[str, ...], ...]

    @property
    def largest_cyclic_component(self) -> tuple[str, ...]:
        return max(self.cyclic_components, key=lambda item: (len(item), item), default=())

    def reverse_sites(self) -> tuple[Site, ...]:
        return tuple(
            site
            for site in self.sites
            if self.root_layers[site.target][0] > self.root_layers[site.source][0]
        )


def _matching(text: str, opening: int, opener: str, closer: str) -> int:
    depth = 0
    for index in range(opening, len(text)):
        if text[index] == opener:
            depth += 1
        elif text[index] == closer:
            depth -= 1
            if depth == 0:
                return index
    raise DiscoveryError(f"unclosed {opener!r} at byte {opening}")


def _item_end(structure: str, start: int) -> int:
    paren = bracket = angle = 0
    for index in range(start, len(structure)):
        char = structure[index]
        if char == "(":
            paren += 1
        elif char == ")" and paren:
            paren -= 1
        elif char == "[":
            bracket += 1
        elif char == "]" and bracket:
            bracket -= 1
        elif char == "<":
            angle += 1
        elif char == ">" and angle:
            angle -= 1
        elif paren == bracket == angle == 0 and char == "{":
            return _matching(structure, index, "{", "}")
        elif paren == bracket == angle == 0 and char in ";,":
            return index
    raise DiscoveryError(f"cannot resolve cfg(test) item at byte {start}")


def mask_cfg_test_nodes(text: str) -> str:
    """Mask syntax nodes proven to be gated by an exact cfg(test)."""
    structure = mask_rust_structure(text)
    chars = list(text)
    cursor = 0
    while match := TEST_CFG_RE.search(structure, cursor):
        node_start = match.end()
        while attribute := re.match(r"\s*#\s*\[[^]]*\]", structure[node_start:]):
            node_start += attribute.end()
        end = _item_end(structure, node_start)
        for index in range(match.start(), end + 1):
            if chars[index] != "\n":
                chars[index] = " "
        cursor = end + 1
    return "".join(chars)


def _module_child_base(path: Path) -> Path:
    return path.parent if path.name == "mod.rs" else path.with_suffix("")


def _resolve_mod_file(parent: Path, name: str, path_attr: str | None) -> Path | None:
    if path_attr is not None:
        candidate = parent.parent / path_attr
        return candidate.resolve() if candidate.is_file() else None
    base = _module_child_base(parent)
    direct = base / f"{name}.rs"
    nested = base / name / "mod.rs"
    if direct.is_file() and nested.is_file():
        raise DiscoveryError(f"ambiguous module files for {parent}: {name}")
    if direct.is_file():
        return direct.resolve()
    if nested.is_file():
        return nested.resolve()
    return None


def _external_module_targets(path: Path, text: str) -> list[tuple[Path, bool]]:
    structure = mask_rust_structure(text)
    targets: list[tuple[Path, bool]] = []
    for match in EXTERNAL_MOD_RE.finditer(structure):
        attrs = text[match.start("attrs") : match.end("attrs")]
        path_match = PATH_ATTR_RE.search(attrs)
        target = _resolve_mod_file(
            path,
            match.group("name"),
            None if path_match is None else path_match.group("path"),
        )
        if target is not None:
            targets.append((target, TEST_CFG_RE.search(mask_rust_structure(attrs)) is not None))
    return targets


def test_only_files(source_files: list[Path]) -> set[Path]:
    """Return files reachable only through a proven cfg(test) module edge."""
    declarations: dict[Path, list[tuple[Path, bool]]] = {}
    for path in source_files:
        targets = _external_module_targets(path, path.read_text(encoding="utf-8"))
        declarations[path.resolve()] = targets

    lib = next((path for path in source_files if path.name == "lib.rs"), None)
    if lib is None:
        raise DiscoveryError("source inventory has no lib.rs entry crate")
    src = lib.parent
    # Entry crates are production roots. Following only non-test module edges
    # proves which files compile without cfg(test), even when their names contain
    # "test". A separate walk from gated edges proves the test-only candidates.
    entries = {
        path.resolve()
        for path in source_files
        if path.name in {"lib.rs", "main.rs"}
        or (
            path.relative_to(src).parts[0] == "bin"
            and (
                len(path.relative_to(src).parts) == 2
                or path.relative_to(src).parts[2:] == ("main.rs",)
            )
        )
    }
    production_reachable = set(entries)
    pending = list(entries)
    while pending:
        parent = pending.pop()
        for target, gated in declarations.get(parent, ()):
            if not gated and target not in production_reachable:
                production_reachable.add(target)
                pending.append(target)

    test_candidates = {
        target
        for targets in declarations.values()
        for target, gated in targets
        if gated
    }
    pending = list(test_candidates)
    while pending:
        parent = pending.pop()
        for target, _ in declarations.get(parent, ()):
            if target not in test_candidates:
                test_candidates.add(target)
                pending.append(target)
    return test_candidates - production_reachable


def _module_components(path: Path, src: Path) -> tuple[str, ...]:
    relative = path.relative_to(src)
    if relative == Path("main.rs"):
        return ("main",)
    if relative.parts[0] == "bin":
        parts = list(relative.with_suffix("").parts)
        if len(parts) == 3 and parts[-1] == "main":
            return ("/".join(parts[:2]),)
        if parts[-1] == "mod":
            parts.pop()
        return ("/".join(parts[:2]), *parts[2:])
    if relative.parts[0] == "migrations":
        return ("migrations",)
    parts = list(relative.with_suffix("").parts)
    if parts[-1] == "mod":
        parts.pop()
    return tuple(parts)


def _source_root(path: Path, src: Path) -> str:
    return _module_components(path, src)[0]


def _inline_modules(structure: str) -> list[tuple[int, int, str]]:
    modules: list[tuple[int, int, str]] = []
    for match in INLINE_MOD_RE.finditer(structure):
        opening = match.end() - 1
        modules.append((opening, _matching(structure, opening, "{", "}"), match.group("name")))
    return modules


def _module_at(
    base: tuple[str, ...], modules: list[tuple[int, int, str]], position: int
) -> tuple[str, ...]:
    enclosing = sorted(
        ((opening, name) for opening, closing, name in modules if opening < position < closing),
        key=lambda item: item[0],
    )
    return (*base, *(name for _, name in enclosing))


def _use_end(structure: str, start: int) -> int:
    braces = parens = brackets = 0
    for index in range(start, len(structure)):
        char = structure[index]
        if char == "{":
            braces += 1
        elif char == "}" and braces:
            braces -= 1
        elif char == "(":
            parens += 1
        elif char == ")" and parens:
            parens -= 1
        elif char == "[":
            brackets += 1
        elif char == "]" and brackets:
            brackets -= 1
        elif char == ";" and braces == parens == brackets == 0:
            return index + 1
    raise DiscoveryError(f"unterminated use declaration at byte {start}")


def _use_paths(statement: str) -> list[tuple[str, ...]]:
    body = re.sub(r"^.*?\buse\b", "", statement, count=1, flags=re.S)
    tokens = re.findall(rf"::|{IDENTIFIER}|[{{}},*]", body)
    index = 0

    def parse_tree(prefix: tuple[str, ...]) -> list[tuple[str, ...]]:
        nonlocal index
        segments: list[str] = []
        while index < len(tokens) and re.fullmatch(IDENTIFIER, tokens[index]):
            segments.append(tokens[index])
            index += 1
            if index >= len(tokens) or tokens[index] != "::":
                break
            index += 1
            if index < len(tokens) and tokens[index] == "{":
                break
        full = (*prefix, *segments)
        paths: list[tuple[str, ...]] = []
        if index < len(tokens) and tokens[index] == "{":
            index += 1
            while index < len(tokens) and tokens[index] != "}":
                paths.extend(parse_tree(full))
                if index < len(tokens) and tokens[index] == ",":
                    index += 1
            if index >= len(tokens):
                raise DiscoveryError("unterminated grouped use")
            index += 1
            return paths
        if index < len(tokens) and tokens[index] == "*":
            index += 1
        if index < len(tokens) and tokens[index] == "as":
            index += 2
        return [full]

    paths: list[tuple[str, ...]] = []
    while index < len(tokens):
        if tokens[index] in ("::", ",", "}"):
            index += 1
            continue
        paths.extend(parse_tree(()))
    return paths


def _resolve_segments(
    segments: tuple[str, ...],
    module: tuple[str, ...],
    known_roots: set[str],
    *,
    library_source: bool,
) -> str | None:
    if not segments:
        return None
    if segments[0] == "remem":
        return segments[1] if len(segments) > 1 and segments[1] in known_roots else None
    if segments[0] == "crate":
        if not library_source:
            return None
        return segments[1] if len(segments) > 1 and segments[1] in known_roots else None
    if segments[0] == "super":
        count = 0
        while count < len(segments) and segments[count] == "super":
            count += 1
        if count >= len(segments) or count > len(module):
            return None
        resolved = (*module[: len(module) - count], segments[count])
        return resolved[0] if resolved and resolved[0] in known_roots else None
    if segments[0] == "self":
        return module[0] if module and module[0] in known_roots else None
    return None


def _normalized_use(statement: str) -> str:
    return re.sub(r"\s+", "", statement)


def _discover_file_sites(
    path: Path,
    root: Path,
    known_roots: set[str],
) -> list[tuple[str, str, str, int, str, str]]:
    src = root / "src"
    raw = path.read_text(encoding="utf-8")
    production = mask_cfg_test_nodes(raw)
    structure = mask_rust_structure(production)
    relative = path.relative_to(root).as_posix()
    source = _source_root(path, src)
    library_source = source not in {"main", "migrations"} and not source.startswith("bin/")
    base_module = _module_components(path, src)
    inline_modules = _inline_modules(structure)
    findings: list[tuple[str, str, str, int, str, str]] = []
    use_spans: list[tuple[int, int]] = []

    for match in USE_START_RE.finditer(structure):
        end = _use_end(structure, match.start())
        use_spans.append((match.start(), end))
        statement = structure[match.start() : end]
        module = _module_at(base_module, inline_modules, match.start())
        targets = {
            target
            for segments in _use_paths(statement)
            if (target := _resolve_segments(
                segments,
                module,
                known_roots,
                library_source=library_source,
            ))
        }
        signature = _normalized_use(statement)
        line = raw.count("\n", 0, match.start()) + 1
        for target in sorted(targets):
            if target != source:
                findings.append((source, target, relative, line, "use", signature))

    def in_use(position: int) -> bool:
        return any(start <= position < end for start, end in use_spans)

    for match in ABSOLUTE_PATH_RE.finditer(structure):
        if in_use(match.start()):
            continue
        if match.group("base") == "crate" and not library_source:
            continue
        target = match.group("root")
        if target in known_roots and target != source:
            spelling = re.sub(r"\s+", "", match.group(0))
            findings.append(
                (source, target, relative, raw.count("\n", 0, match.start()) + 1, "path", spelling)
            )

    for match in RELATIVE_PATH_RE.finditer(structure):
        if in_use(match.start()):
            continue
        segments = tuple(re.findall(IDENTIFIER, match.group(0)))
        target = _resolve_segments(
            segments,
            _module_at(base_module, inline_modules, match.start()),
            known_roots,
            library_source=library_source,
        )
        if target is not None and target != source:
            findings.append(
                (
                    source,
                    target,
                    relative,
                    raw.count("\n", 0, match.start()) + 1,
                    "relative",
                    re.sub(r"\s+", "", match.group(0)),
                )
            )
    return findings


def _cyclic_components(roots: set[str], edges: set[tuple[str, str]]) -> tuple[tuple[str, ...], ...]:
    graph: dict[str, list[str]] = {root: [] for root in roots}
    for source, target in edges:
        if source != target:
            graph[source].append(target)
    index = 0
    indices: dict[str, int] = {}
    lowlinks: dict[str, int] = {}
    stack: list[str] = []
    on_stack: set[str] = set()
    components: list[tuple[str, ...]] = []

    def visit(node: str) -> None:
        nonlocal index
        indices[node] = lowlinks[node] = index
        index += 1
        stack.append(node)
        on_stack.add(node)
        for target in graph[node]:
            if target not in indices:
                visit(target)
                lowlinks[node] = min(lowlinks[node], lowlinks[target])
            elif target in on_stack:
                lowlinks[node] = min(lowlinks[node], indices[target])
        if lowlinks[node] == indices[node]:
            component: list[str] = []
            while True:
                member = stack.pop()
                on_stack.remove(member)
                component.append(member)
                if member == node:
                    break
            if len(component) > 1:
                components.append(tuple(sorted(component)))

    for root in sorted(roots):
        if root not in indices:
            visit(root)
    return tuple(sorted(components, key=lambda item: (len(item), item)))


def scan_repository(
    root: Path,
    *,
    configured_layers: dict[str, tuple[int, str]] | None = None,
) -> ScanResult:
    src = root / "src"
    lib = src / "lib.rs"
    if not lib.is_file():
        raise DiscoveryError(f"missing {lib}")
    configured_names = [name for _, roots in LAYER_ROOTS for name in roots]
    duplicate_names = sorted({name for name in configured_names if configured_names.count(name) > 1})
    if configured_layers is None and duplicate_names:
        raise DiscoveryError("roots assigned to multiple layers: " + ", ".join(duplicate_names))
    layer_map = dict(ROOT_LAYERS if configured_layers is None else configured_layers)
    lib_structure = mask_rust_structure(mask_cfg_test_nodes(lib.read_text(encoding="utf-8")))
    discovered_roots = set(ROOT_MOD_RE.findall(lib_structure))
    configured_roots = set(layer_map)
    unknown = sorted(discovered_roots - configured_roots)
    stale = sorted(configured_roots - discovered_roots)
    if unknown or stale:
        details = []
        if unknown:
            details.append("unclassified roots: " + ", ".join(unknown))
        if stale:
            details.append("configured roots absent from src/lib.rs: " + ", ".join(stale))
        raise DiscoveryError("; ".join(details))

    source_files = sorted(src.rglob("*.rs"))
    excluded = test_only_files(source_files)
    special_layers = {
        "main": (4, "adapters"),
        "migrations": (1, "storage"),
    }
    for path in source_files:
        source = _source_root(path, src)
        if source.startswith("bin/"):
            special_layers[source] = (4, "adapters")
    all_layers = {**layer_map, **special_layers}
    raw_findings: list[tuple[str, str, str, int, str, str]] = []
    for path in source_files:
        if path == lib or path.resolve() in excluded:
            continue
        source = _source_root(path, src)
        if source not in all_layers:
            raise DiscoveryError(f"{path.relative_to(root)} belongs to unknown root {source}")
        try:
            raw_findings.extend(_discover_file_sites(path, root, configured_roots))
        except DiscoveryError as error:
            raise DiscoveryError(f"{path.relative_to(root)}: {error}") from error

    counters: defaultdict[tuple[str, str, str, str, str], int] = defaultdict(int)
    sites: list[Site] = []
    for source, target, path, line, kind, signature in sorted(
        raw_findings,
        key=lambda item: (item[2], item[3], item[0], item[1], item[4], item[5]),
    ):
        counter_key = (source, target, path, kind, signature)
        counters[counter_key] += 1
        sites.append(
            Site(source, target, path, line, kind, signature, counters[counter_key])
        )
    edges = {(site.source, site.target) for site in sites}
    return ScanResult(all_layers, tuple(sites), _cyclic_components(set(all_layers), edges))


def site_digest(site: Site) -> str:
    rendered = "\0".join(map(str, site.key()))
    return hashlib.sha256(rendered.encode("utf-8")).hexdigest()[:16]
