#!/usr/bin/env python3
"""Cross-target and implementation evidence for the GH969 surface manifest."""

from __future__ import annotations

import hashlib
import html
import re
import tomllib
from pathlib import Path


EXPERIMENTAL_CALLER_SYMBOLS = {
    "mcp-context-bundle": ("fn context_bundle(", ".context_bundle("),
    "rust-context-bundle": ("crate::context_bundle",),
    "retrieval-router-plan": ("crate::retrieval_router",),
    "rust-retrieval-router": ("crate::retrieval_router",),
    "routed-search-parameters": (
        "compile_search_retrieval_plan",
        "params.task_intent",
        "params.token_budget",
        "params.include_superseded",
    ),
    "entity-bfs": ("retrieval::entity", "entity_graph"),
    "coding-public-benchmarks": ("coding_bench",),
}


def expanded_default_features(root: Path) -> set[str]:
    cargo = tomllib.loads((root / "Cargo.toml").read_text(encoding="utf-8"))
    features = cargo.get("features", {})
    defaults = features.get("default")
    if not isinstance(features, dict) or not isinstance(defaults, list):
        raise RuntimeError("Cargo.toml [features].default must be a string array")
    active: set[str] = set()
    queue = list(defaults)
    while queue:
        feature = queue.pop()
        if not isinstance(feature, str):
            raise RuntimeError("Cargo.toml feature members must be strings")
        if feature in active or feature.startswith("dep:") or "/" in feature:
            continue
        active.add(feature)
        dependencies = features.get(feature, [])
        if not isinstance(dependencies, list):
            raise RuntimeError(f"Cargo.toml feature {feature!r} must be a string array")
        queue.extend(dependencies)
    return active


_PUBLIC_DECLARATION = re.compile(
    r"\bpub\s+(?!\()(?P<kind>async\s+fn|fn|struct|enum|trait|type|const|static|mod|use)\s+"
    r"(?P<name>[A-Za-z_][A-Za-z0-9_]*)"
)


def _matching_brace(text: str, opening: int) -> int:
    depth = 0
    for index in range(opening, len(text)):
        depth += text[index] == "{"
        depth -= text[index] == "}"
        if depth == 0:
            return index
    raise RuntimeError("unclosed Rust declaration body")


def _source_signature(text: str, match: re.Match[str]) -> str:
    """Fingerprint a public declaration while excluding function implementation bodies."""
    cursor = match.end()
    paren = bracket = angle = 0
    while cursor < len(text):
        char = text[cursor]
        paren += char == "("
        paren -= char == ")"
        bracket += char == "["
        bracket -= char == "]"
        angle += char == "<"
        angle -= char == ">" and angle > 0
        if not (paren or bracket or angle) and char in "{;":
            break
        cursor += 1
    if cursor == len(text):
        raise RuntimeError(f"cannot terminate public declaration {match.group('name')!r}")
    declaration = text[match.start() : cursor]
    kind = re.sub(r"\s+", "_", match.group("kind"))
    if text[cursor] == "{" and kind in {"struct", "enum", "trait"}:
        closing = _matching_brace(text, cursor)
        body = text[cursor + 1 : closing]
        if kind == "struct":
            body = " ".join(field.group(0) for field in re.finditer(r"\bpub\s+(?:[A-Za-z_][A-Za-z0-9_]*\s*:|\([^)]*\))[^,}]*", body))
        declaration += "{" + body + "}"
    normalized = re.sub(r"\s+", " ", declaration).strip()
    return hashlib.sha256(normalized.encode()).hexdigest()


def discover_target_gated_exports(root: Path) -> set[str]:
    """Inventory target-only signatures reachable through public modules from lib.rs."""
    exports: set[str] = set()
    gated = re.compile(
        r"#\s*\[\s*cfg\s*\((?P<cfg>[^]]*(?:windows|unix|target_(?:os|arch|env))[^]]*)\)\s*\]"
        r"(?:\s*#\s*\[[^]]*\])*\s*(?P<declaration>pub\s+(?!\()"
        r"(?P<kind>async\s+fn|fn|struct|enum|trait|type|const|static|mod|use)\s+"
        r"(?P<name>[A-Za-z_][A-Za-z0-9_]*))",
        re.S,
    )
    gated_impl = re.compile(
        r"#\s*\[\s*cfg\s*\((?P<cfg>[^]]*(?:windows|unix|target_(?:os|arch|env))[^]]*)\)\s*\]"
        r"(?:\s*#\s*\[[^]]*\])*\s*impl(?:<[^>{}]*>)?\s+(?P<owner>[^{}]+?)\s*\{",
        re.S,
    )
    any_impl = re.compile(r"\bimpl(?:<[^>{}]*>)?\s+(?P<owner>[^{}]+?)\s*\{", re.S)

    def column_zero(text: str, position: int) -> bool:
        return position == 0 or text.rfind("\n", 0, position) + 1 == position

    def combined_cfg(parent: str | None, child: str) -> str:
        return child if parent is None else f"all({parent},{child})"

    def module_file(parent: Path, name: str) -> Path | None:
        base = parent.parent / parent.stem if parent.name not in {"lib.rs", "mod.rs"} else parent.parent
        candidates = (base / f"{name}.rs", base / name / "mod.rs")
        return next((candidate for candidate in candidates if candidate.is_file()), None)

    def add_impl_items(raw: str, relative: str, cfg: str, match: re.Match[str]) -> None:
        owner = re.sub(r"\s+", "", match.group("owner"))
        closing = _matching_brace(raw, match.end() - 1)
        body = raw[match.end() : closing]
        for item in _PUBLIC_DECLARATION.finditer(body):
            kind = re.sub(r"\s+", "_", item.group("kind"))
            digest = _source_signature(body, item)
            exports.add(f"{relative}::{cfg}::impl:{owner}::{kind}:{item.group('name')}::sha256={digest}")

    def visit(path: Path, inherited_cfg: str | None, prefix: str, seen: set[tuple[Path, str | None]]) -> None:
        key = (path, inherited_cfg)
        if key in seen:
            return
        seen.add(key)
        raw = path.read_text(encoding="utf-8")
        relative = path.relative_to(root).as_posix()
        gated_declarations = [match for match in gated.finditer(raw) if column_zero(raw, match.start())]
        gated_starts = {match.start("declaration") for match in gated_declarations}
        module_cfgs: dict[int, str] = {}
        for match in gated_declarations:
            cfg = combined_cfg(inherited_cfg, re.sub(r"\s+", "", match.group("cfg")))
            kind = re.sub(r"\s+", "_", match.group("kind"))
            name = match.group("name")
            declaration = _PUBLIC_DECLARATION.search(raw, match.start("declaration"))
            if declaration is None:
                raise RuntimeError(f"cannot resolve target-gated declaration {name!r}")
            digest = _source_signature(raw, declaration)
            exports.add(f"{relative}::{cfg}::{prefix}{kind}:{name}::sha256={digest}")
            if kind == "mod":
                module_cfgs[declaration.start()] = cfg
                child = module_file(path, name)
                if child is not None:
                    visit(child, cfg, f"{prefix}{name}::", seen)
                elif raw[declaration.end() :].lstrip().startswith(";"):
                    raise RuntimeError(f"cannot resolve target-gated public module {name!r} from {relative}")

        if inherited_cfg is not None:
            for match in _PUBLIC_DECLARATION.finditer(raw):
                if not column_zero(raw, match.start()) or match.start() in gated_starts:
                    continue
                kind = re.sub(r"\s+", "_", match.group("kind"))
                digest = _source_signature(raw, match)
                exports.add(f"{relative}::{inherited_cfg}::{prefix}{kind}:{match.group('name')}::sha256={digest}")

        gated_impl_starts: set[int] = set()
        for match in gated_impl.finditer(raw):
            if not column_zero(raw, match.start()):
                continue
            impl_start = raw.rfind("impl", match.start(), match.end())
            if impl_start < 0:
                raise RuntimeError("cannot resolve target-gated impl declaration")
            gated_impl_starts.add(impl_start)
            cfg = combined_cfg(inherited_cfg, re.sub(r"\s+", "", match.group("cfg")))
            add_impl_items(raw, relative, cfg, match)
        if inherited_cfg is not None:
            for match in any_impl.finditer(raw):
                if column_zero(raw, match.start()) and match.start() not in gated_impl_starts:
                    add_impl_items(raw, relative, inherited_cfg, match)

        for declaration in _PUBLIC_DECLARATION.finditer(raw):
            if not column_zero(raw, declaration.start()) or declaration.group("kind") != "mod":
                continue
            name = declaration.group("name")
            effective_cfg = module_cfgs.get(declaration.start(), inherited_cfg)
            child = module_file(path, name)
            if child is not None:
                visit(child, effective_cfg, f"{prefix}{name}::", seen)
                continue
            tail = raw[declaration.end() :].lstrip()
            if tail.startswith(";"):
                raise RuntimeError(f"cannot resolve public module {name!r} from {relative}")
            if tail.startswith("{"):
                closing = _matching_brace(tail, 0)
                body = tail[1:closing]
                if effective_cfg is not None or re.search(r"#\s*\[\s*cfg\s*\([^]]*(?:windows|unix|target_)", body):
                    raise RuntimeError(f"target-aware discovery does not support inline public module {name!r} in {relative}")

    visit(root / "src/lib.rs", None, "", set())
    return exports


def rustdoc_signature(page_text: str, anchor: str | None = None) -> str:
    """Hash a normalized public declaration rendered by rustdoc."""
    if anchor is None or anchor.startswith(("structfield.", "variant.")):
        match = re.search(r'<pre class="rust item-decl"><code>(.*?)</code></pre>', page_text, re.S)
    else:
        start = page_text.find(f'id="{anchor}"')
        end = page_text.find("</section>", start)
        match = None if start < 0 or end < 0 else re.search(
            r'<h4 class="code-header">(.*?)</h4>', page_text[start:end], re.S
        )
    if not match:
        raise RuntimeError(f"rustdoc public signature is missing for {anchor or 'item'}")
    signature = html.unescape(re.sub(r"<[^>]+>", "", match.group(1)))
    normalized = re.sub(r"\s+", " ", signature).strip()
    return hashlib.sha256(normalized.encode()).hexdigest()


def discover_product_rows(root: Path) -> dict[str, dict[str, str]]:
    product = (root / "docs/specs/GH969/PRODUCT.md").read_text(encoding="utf-8")
    section = product.split("## Canonical Surface Inventory", 1)
    if len(section) != 2:
        raise RuntimeError("GH969 PRODUCT is missing Canonical Surface Inventory")
    table = section[1].split("## Decision Gates", 1)[0]
    keys = ("entry", "owner", "status", "real_caller_default", "evidence", "compatibility", "next_decision")
    rows: dict[str, dict[str, str]] = {}
    for line in table.splitlines():
        columns = [column.strip() for column in line.strip().strip("|").split("|")]
        if len(columns) != 8 or not columns[0].startswith("`"):
            continue
        name = columns[0].strip("`")
        if name in rows:
            raise RuntimeError(f"duplicate GH969 PRODUCT inventory row {name!r}")
        values = dict(zip(keys, columns[1:], strict=True))
        values["status"] = values["status"].strip("`")
        rows[name] = values
    if not rows:
        raise RuntimeError("GH969 PRODUCT canonical table has no keyed lifecycle rows")
    return rows


PRODUCTION_DEFAULT_GUARDS = {
    "sessionstart-context-bundle": "context_bundle_default",
    "currenttruth-v1": "current_truth_default",
    "graph-edges": "positive_graph_weight",
    "legacy-events": "legacy_events_projection",
}


def build_default_guard(root: Path, mode: str) -> dict[str, object]:
    if mode == "context_bundle_default":
        path = root / "src/context/render_bundle.rs"
        raw = path.read_text(encoding="utf-8")
        markers = ('"" | "bundle" => Ok(ContextBundleRenderMode::Bundle)', "NotPresent) => Ok(ContextBundleRenderMode::Bundle)")
        if not all(marker in raw for marker in markers):
            raise RuntimeError("SessionStart Context Bundle is no longer the implementation default")
        value: object = "bundle"
    elif mode == "current_truth_default":
        path = root / "src/context_bundle/compile.rs"
        compile_source = path.read_text(encoding="utf-8")
        query_source = (root / "src/context/query.rs").read_text(encoding="utf-8")
        markers = (
            "crate::context_bundle::project_for_scope(",
            "current_truth_projection",
            "let Some(projection) = current_truth_projection else",
            "attach_shadow_comparison(&mut bundle, &projection)",
            "activate_current_truth_channel(",
        )
        raw = query_source + "\n" + compile_source
        if not all(marker in raw for marker in markers):
            raise RuntimeError("CurrentTruth is no longer projected and activated on the default Context Bundle path")
        value = "projected-and-activated"
    elif mode == "positive_graph_weight":
        path = root / "src/retrieval/search/memory/weights.rs"
        raw = path.read_text(encoding="utf-8")
        match = re.search(r"\bconst\s+GRAPH_WEIGHT\s*:\s*f64\s*=\s*([0-9]+(?:\.[0-9]+)?)\s*;", raw)
        if not match or float(match.group(1)) <= 0:
            raise RuntimeError("production GRAPH_WEIGHT must remain non-zero")
        value = float(match.group(1))
    elif mode == "legacy_events_projection":
        path = root / "src/memory/events/write.rs"
        write_source = path.read_text(encoding="utf-8")
        cursor_source = (root / "src/observe/cursor.rs").read_text(encoding="utf-8")
        hook_source = (root / "src/observe/hook.rs").read_text(encoding="utf-8")
        caller_sources = [cursor_source, hook_source]
        raw = "\n".join([write_source, *caller_sources])
        writer_markers = (
            "pub(crate) fn insert_event_for_capture(",
            "pub(crate) fn replace_event_for_capture(",
            "ON CONFLICT(captured_event_id)",
        )
        caller_markers = (
            "crate::memory::insert_event_for_capture(",
            "crate::memory::replace_event_for_capture(",
        )
        if (
            not all(marker in write_source for marker in writer_markers)
            or not all(marker in cursor_source for marker in caller_markers)
            or caller_markers[0] not in hook_source
        ):
            raise RuntimeError("legacy events projection no longer has both transactional writers and capture callers")
        value = "transactional-insert-and-replace"
    else:
        raise RuntimeError(f"unknown production default guard {mode!r}")
    return {
        "mode": mode,
        "path": path.relative_to(root).as_posix(),
        "value": value,
        "sha256": hashlib.sha256(raw.encode()).hexdigest(),
    }


def discover_search_parameters(root: Path) -> set[str]:
    text = (root / "src/mcp/types.rs").read_text(encoding="utf-8")
    match = re.search(r"\bstruct\s+SearchParams\s*\{(?P<body>.*?)\}", text, re.S)
    if not match:
        raise RuntimeError("cannot resolve MCP SearchParams served-schema source")
    fields = set(re.findall(r"\bpub\s+(?:r#)?([A-Za-z_][A-Za-z0-9_]*)\s*:", match.group("body")))
    if not fields:
        raise RuntimeError("MCP SearchParams has no discoverable served fields")
    return {f"search.{field}" for field in fields}


def _production_sources(root: Path) -> list[Path]:
    paths: list[Path] = []
    for path in sorted((root / "src").rglob("*.rs")):
        if (
            "tests" in path.parts
            or path.name == "test_support.rs"
            or path.stem.startswith("tests")
            or path.stem.endswith("_tests")
        ):
            continue
        paths.append(path)
    return paths


def build_caller_guard(root: Path, symbols: tuple[str, ...]) -> dict[str, object]:
    callers: list[dict[str, str]] = []
    for path in _production_sources(root):
        raw = path.read_text(encoding="utf-8")
        source = re.sub(r"//[^\n]*|/\*.*?\*/", "", raw, flags=re.S)
        if any(symbol in source for symbol in symbols):
            callers.append({
                "path": path.relative_to(root).as_posix(),
                "sha256": hashlib.sha256(raw.encode()).hexdigest(),
            })
    return {"symbols": list(symbols), "callers": callers}


def offline_categories(root: Path, roots: list[str]) -> dict[str, list[str]]:
    files = sorted(
        path.relative_to(root).as_posix()
        for declared_root in roots
        for path in (root / declared_root).rglob("*")
        if path.is_file()
    )
    categories = {name: [] for name in ("executables", "schemas", "fixtures", "data", "documents")}
    for path in files:
        if "/scripts/" in path:
            if not path.endswith(".py"):
                raise RuntimeError(f"unsupported offline script artifact {path!r}")
            category = "executables"
        elif "/schemas/" in path:
            if not path.endswith(".json"):
                raise RuntimeError(f"unsupported offline schema artifact {path!r}")
            category = "schemas"
        elif "/examples/" in path:
            category = "fixtures"
        elif "/tasks/" in path or path.endswith("benchmark-charter.json"):
            category = "data"
        else:
            category = "documents"
        categories[category].append(path)
    classified = [path for values in categories.values() for path in values]
    if sorted(classified) != files or len(classified) != len(set(classified)):
        raise RuntimeError("every offline artifact must resolve to exactly one category")
    return categories
