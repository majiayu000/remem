#!/usr/bin/env python3
"""Cross-target and implementation evidence for the GH969 surface manifest."""

from __future__ import annotations

import hashlib
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


def discover_target_gated_exports(root: Path) -> set[str]:
    """Conservatively inventory public declarations behind platform cfgs on any host."""
    exports: set[str] = set()
    declaration = re.compile(
        r"#\s*\[\s*cfg\s*\((?P<cfg>[^]]*(?:windows|unix|target_(?:os|arch|env))[^]]*)\)\s*\]"
        r"(?:\s*#\s*\[[^]]*\])*\s*pub\s+(?!\()"
        r"(?P<kind>async\s+fn|fn|struct|enum|trait|type|const|static|mod|use)\s+"
        r"(?P<name>[A-Za-z_][A-Za-z0-9_]*)",
        re.S,
    )
    for path in sorted((root / "src").rglob("*.rs")):
        raw = path.read_text(encoding="utf-8")
        digest = hashlib.sha256(raw.encode()).hexdigest()
        relative = path.relative_to(root).as_posix()
        for match in declaration.finditer(raw):
            cfg = re.sub(r"\s+", "", match.group("cfg"))
            kind = re.sub(r"\s+", "_", match.group("kind"))
            exports.add(f"{relative}::{cfg}::{kind}:{match.group('name')}::{digest}")
    return exports


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
    return {
        "executables": [path for path in files if "/scripts/" in path and path.endswith(".py")],
        "schemas": [path for path in files if "/schemas/" in path and path.endswith(".json")],
        "fixtures": [path for path in files if "/examples/" in path],
        "data": [path for path in files if "/tasks/" in path or path.endswith("benchmark-charter.json")],
        "documents": [
            path for path in files
            if not any(part in path for part in ("/scripts/", "/schemas/", "/examples/", "/tasks/"))
            and not path.endswith("benchmark-charter.json")
        ],
    }
