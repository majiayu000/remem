#!/usr/bin/env python3
"""Discover callable remem surfaces from their real export/registration roots."""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import tomllib
from html.parser import HTMLParser
from pathlib import Path


HTTP_METHODS = ("delete", "get", "head", "options", "patch", "post", "put", "trace")
ROOT = Path(__file__).resolve().parents[2]


def _matching(text: str, opening: int, left: str, right: str) -> int | None:
    depth = 0
    quote: str | None = None
    escaped = False
    line_comment = False
    block_depth = 0
    index = opening
    while index < len(text):
        char = text[index]
        following = text[index + 1] if index + 1 < len(text) else ""
        if line_comment:
            if char == "\n":
                line_comment = False
        elif block_depth:
            if char == "/" and following == "*":
                block_depth += 1
                index += 1
            elif char == "*" and following == "/":
                block_depth -= 1
                index += 1
        elif quote:
            if escaped:
                escaped = False
            elif char == "\\":
                escaped = True
            elif char == quote:
                quote = None
        elif char == "/" and following == "/":
            line_comment = True
            index += 1
        elif char == "/" and following == "*":
            block_depth = 1
            index += 1
        elif char in {'"', "'"}:
            quote = char
        elif char == left:
            depth += 1
        elif char == right:
            depth -= 1
            if depth == 0:
                return index
        index += 1
    return None


def _top_level_parts(text: str, separator: str = ",") -> list[str]:
    parts: list[str] = []
    start = 0
    depths = {"(": 0, "[": 0, "{": 0, "<": 0}
    closes = {")": "(", "]": "[", "}": "{", ">": "<"}
    quote: str | None = None
    escaped = False
    line_comment = False
    block_depth = 0
    index = 0
    while index < len(text):
        char = text[index]
        following = text[index + 1] if index + 1 < len(text) else ""
        if line_comment:
            if char == "\n":
                line_comment = False
        elif block_depth:
            if char == "/" and following == "*":
                block_depth += 1
                index += 1
            elif char == "*" and following == "/":
                block_depth -= 1
                index += 1
        elif quote:
            if escaped:
                escaped = False
            elif char == "\\":
                escaped = True
            elif char == quote:
                quote = None
        elif char == "/" and following == "/":
            line_comment = True
            index += 1
        elif char == "/" and following == "*":
            block_depth = 1
            index += 1
        elif char in {'"', "'"}:
            quote = char
        elif char in depths:
            depths[char] += 1
        elif char in closes and depths[closes[char]]:
            depths[closes[char]] -= 1
        elif char == separator and not any(depths.values()):
            parts.append(text[start:index])
            start = index + 1
        index += 1
    parts.append(text[start:])
    return parts


class _AllItemsParser(HTMLParser):
    def __init__(self) -> None:
        super().__init__()
        self.in_all_items = False
        self.list_depth = 0
        self.href: str | None = None
        self.anchor_text: list[str] = []
        self.items: list[str] = []

    def handle_starttag(self, tag: str, attrs: list[tuple[str, str | None]]) -> None:
        attributes = dict(attrs)
        if tag == "ul" and attributes.get("class") == "all-items":
            self.in_all_items = True
            self.list_depth = 1
            return
        if self.in_all_items and tag == "ul":
            self.list_depth += 1
        if self.in_all_items and tag == "a":
            self.href = attributes.get("href")
            self.anchor_text = []

    def handle_data(self, data: str) -> None:
        if self.href is not None:
            self.anchor_text.append(data)

    def handle_endtag(self, tag: str) -> None:
        if self.in_all_items and tag == "a" and self.href is not None:
            item = "".join(self.anchor_text).strip()
            if item:
                self.items.append(item)
            self.href = None
        if self.in_all_items and tag == "ul":
            self.list_depth -= 1
            if self.list_depth == 0:
                self.in_all_items = False


def discover_rust_exports(root: Path, *, doc_root: Path | None = None) -> set[str]:
    """Use rustdoc's compiler-resolved public graph, including public re-exports."""
    if doc_root is None:
        result = subprocess.run(
            ["cargo", "doc", "--locked", "--quiet", "--no-deps", "--lib"],
            cwd=root,
            text=True,
            capture_output=True,
            check=False,
        )
        if result.returncode != 0:
            detail = (result.stderr or result.stdout).strip()
            raise RuntimeError(f"cargo doc failed while discovering Rust exports: {detail}")
        doc_root = root / "target/doc/remem"

    all_items = doc_root / "all.html"
    if not all_items.is_file():
        raise RuntimeError(f"rustdoc public-item index is missing: {all_items}")
    parser = _AllItemsParser()
    parser.feed(all_items.read_text(encoding="utf-8"))
    if not parser.items:
        raise RuntimeError(f"rustdoc public-item index has no all-items list: {all_items}")

    exports = {f"remem::{item}" for item in parser.items}
    root_index = doc_root / "index.html"
    queue = [root_index]
    visited: set[Path] = set()
    module_link = re.compile(r'<a\s+class="[^"]*\bmod\b[^"]*"\s+href="([^"]+/index\.html)"')
    while queue:
        page = queue.pop()
        if page in visited:
            continue
        visited.add(page)
        if not page.is_file():
            raise RuntimeError(f"rustdoc linked public module page is missing: {page}")
        for href in module_link.findall(page.read_text(encoding="utf-8")):
            child = (page.parent / href).resolve()
            try:
                relative = child.relative_to(doc_root.resolve())
            except ValueError as exc:
                raise RuntimeError(f"rustdoc module link escapes crate docs: {href}") from exc
            exports.add("remem::" + "::".join(relative.parts[:-1]))
            queue.append(child)
    return exports


def discover_mcp_tools(root: Path) -> tuple[set[str], dict[str, str]]:
    path = root / "src/mcp/server/tool_contracts.rs"
    text = path.read_text(encoding="utf-8")
    match = re.search(r"const\s+CONTRACTS\s*:[^=]+?=\s*\[(?P<body>.*?)\n\];", text, re.S)
    if not match:
        raise RuntimeError(f"cannot resolve MCP CONTRACTS registry in {path}")
    body = match.group("body")
    tools: dict[str, str] = {}
    constructor_re = re.compile(
        r"json_(?:object|array)\s*\(\s*\"(?P<name>[^\"]+)\"\s*,\s*\"(?P<title>[^\"]+)\"",
        re.S,
    )
    literal_re = re.compile(
        r"ToolContract\s*\{.*?name:\s*\"(?P<name>[^\"]+)\"\s*,\s*"
        r"title:\s*\"(?P<title>[^\"]+)\"",
        re.S,
    )
    for registry_match in (*constructor_re.finditer(body), *literal_re.finditer(body)):
        tools[registry_match.group("name")] = registry_match.group("title")
    declared = re.search(r"CONTRACTS\s*:\s*\[ToolContract;\s*(\d+)\]", text)
    if not declared or len(tools) != int(declared.group(1)):
        expected = declared.group(1) if declared else "unknown"
        raise RuntimeError(
            f"MCP registry discovery found {len(tools)} tools but CONTRACTS declares {expected}"
        )
    return set(tools), tools


def discover_rest_routes(root: Path) -> set[str]:
    path = root / "src/api/server.rs"
    text = path.read_text(encoding="utf-8")
    routes: set[str] = set()
    cursor = 0
    while True:
        opening = text.find(".route(", cursor)
        if opening < 0:
            break
        opening += len(".route")
        closing = _matching(text, opening, "(", ")")
        if closing is None:
            raise RuntimeError(f"unclosed .route call in {path}")
        arguments = _top_level_parts(text[opening + 1 : closing])
        if len(arguments) < 2:
            raise RuntimeError(f"route call has fewer than two arguments in {path}")
        path_match = re.fullmatch(r'\s*"([^\"]+)"\s*', arguments[0])
        if not path_match:
            raise RuntimeError(f"route path is not a literal in {path}: {arguments[0].strip()}")
        methods = {
            method.upper()
            for method in HTTP_METHODS
            if re.search(rf"\b{method}\s*\(", arguments[1])
        }
        if not methods:
            raise RuntimeError(f"route has no recognized HTTP method in {path}: {arguments[1].strip()}")
        routes.update(f"{method} {path_match.group(1)}" for method in methods)
        cursor = closing + 1
    if not routes:
        raise RuntimeError(f"no REST routes discovered in {path}")
    return routes


def _strip_leading_attributes(text: str) -> str:
    value = text.lstrip()
    while value.startswith("#"):
        bracket = value.find("[")
        if bracket < 0:
            break
        closing = _matching(value, bracket, "[", "]")
        if closing is None:
            break
        value = value[closing + 1 :].lstrip()
    return value


def _mask_comments(text: str) -> str:
    chars = list(text)
    quote: str | None = None
    escaped = False
    index = 0
    while index < len(text):
        char = text[index]
        following = text[index + 1] if index + 1 < len(text) else ""
        if quote:
            if escaped:
                escaped = False
            elif char == "\\":
                escaped = True
            elif char == quote:
                quote = None
        elif char in {'"', "'"}:
            quote = char
        elif char == "/" and following == "/":
            end = text.find("\n", index + 2)
            end = len(text) if end < 0 else end
            chars[index:end] = " " * (end - index)
            index = end - 1
        elif char == "/" and following == "*":
            depth = 1
            end = index + 2
            while end < len(text) and depth:
                pair = text[end : end + 2]
                if pair == "/*":
                    depth += 1
                    end += 2
                elif pair == "*/":
                    depth -= 1
                    end += 2
                else:
                    end += 1
            chars[index:end] = " " * (end - index)
            index = end - 1
        index += 1
    return "".join(chars)


def _kebab_case(name: str) -> str:
    value = re.sub(r"([A-Z]+)([A-Z][a-z])", r"\1-\2", name)
    value = re.sub(r"([a-z0-9])([A-Z])", r"\1-\2", value)
    return value.replace("_", "-").lower()


def _subcommand_enums(root: Path) -> dict[str, list[tuple[str, str | None]]]:
    enums: dict[str, list[tuple[str, str | None]]] = {}
    for path in sorted((root / "src/cli").glob("*.rs")):
        text = path.read_text(encoding="utf-8")
        enum_re = re.compile(
            r"#\s*\[\s*derive\s*\([^]]*\bSubcommand\b[^]]*\)\s*\]"
            r"(?:(?:\s|#\s*\[[^]]*\])*)"
            r"(?:pub(?:\s*\([^)]*\))?\s+)?enum\s+(?P<name>[A-Za-z_][A-Za-z0-9_]*)\s*\{",
            re.S,
        )
        for match in enum_re.finditer(text):
            opening = match.end() - 1
            closing = _matching(text, opening, "{", "}")
            if closing is None:
                raise RuntimeError(f"unclosed Subcommand enum {match.group('name')} in {path}")
            variants: list[tuple[str, str | None]] = []
            for raw_variant in _top_level_parts(text[opening + 1 : closing]):
                if not raw_variant.strip():
                    continue
                visible = _strip_leading_attributes(_mask_comments(raw_variant))
                variant_match = re.match(r"([A-Za-z_][A-Za-z0-9_]*)", visible)
                if not variant_match:
                    continue
                variant = variant_match.group(1)
                override = re.search(
                    r"#\s*\[\s*command\s*\([^]]*?\bname\s*=\s*\"([^\"]+)\"",
                    raw_variant,
                    re.S,
                )
                command_name = override.group(1) if override else _kebab_case(variant)
                child: str | None = None
                child_match = re.search(
                    r"#\s*\[\s*command\s*\(\s*subcommand\s*\)\s*\]"
                    r"\s*[A-Za-z_][A-Za-z0-9_]*\s*:\s*"
                    r"(?:Option\s*<\s*)?(?P<type>[A-Za-z_][A-Za-z0-9_:]*)",
                    raw_variant,
                    re.S,
                )
                if child_match:
                    child = child_match.group("type").split("::")[-1]
                variants.append((command_name, child))
            enums[match.group("name")] = variants
    return enums


def discover_cli_commands(root: Path) -> set[str]:
    enums = _subcommand_enums(root)
    if "Commands" not in enums:
        raise RuntimeError("cannot resolve root Clap Commands enum under src/cli")
    commands: set[str] = set()

    def visit(enum_name: str, prefix: tuple[str, ...], ancestors: tuple[str, ...]) -> None:
        if enum_name in ancestors:
            raise RuntimeError(f"recursive Clap subcommand graph: {' -> '.join((*ancestors, enum_name))}")
        if enum_name not in enums:
            raise RuntimeError(f"Clap subcommand enum {enum_name} is referenced but not discovered")
        commands.add("remem " + " ".join((*prefix, "help")))
        for command, child in enums[enum_name]:
            path = (*prefix, command)
            commands.add("remem " + " ".join(path))
            if child:
                visit(child, path, (*ancestors, enum_name))

    visit("Commands", (), ())
    return commands


def discover_default_features(root: Path) -> set[str]:
    cargo = tomllib.loads((root / "Cargo.toml").read_text(encoding="utf-8"))
    features = cargo.get("features", {})
    defaults = features.get("default")
    if not isinstance(defaults, list) or not all(isinstance(item, str) for item in defaults):
        raise RuntimeError("Cargo.toml [features].default must be a string array")
    return set(defaults)


def discover_all(root: Path, *, doc_root: Path | None = None) -> dict[str, set[str]]:
    mcp_tools, _ = discover_mcp_tools(root)
    return {
        "rust_export": discover_rust_exports(root, doc_root=doc_root),
        "mcp_tool": mcp_tools,
        "rest_route": discover_rest_routes(root),
        "cli_command": discover_cli_commands(root),
        "default_feature": discover_default_features(root),
    }


ROW_STATUS = {
    "rust-library": "production",
    "mcp-production": "production",
    "rest-api": "production",
    "cli-production": "production",
    "sessionstart-context-bundle": "production",
    "mcp-context-bundle": "experimental",
    "rust-context-bundle": "experimental",
    "currenttruth-v1": "production",
    "doctor-truth": "production",
    "retrieval-router-plan": "experimental",
    "rust-retrieval-router": "experimental",
    "routed-search-parameters": "experimental",
    "graph-edges": "production",
    "entity-bfs": "experimental",
    "local-onnx": "production",
    "deterministic-eval": "production",
    "coding-public-benchmarks": "experimental",
    "legacy-pending": "recovery-only",
    "legacy-events": "deprecated",
    "historical-summary": "recovery-only",
    "currenttruth-v2": "spec-only",
    "cross-host-harness": "experimental",
    "cross-host-completion": "spec-only",
}

ROW_DETAILS: dict[str, tuple[str, list[str], str, list[str], list[str], str]] = {
    "rust-library": ("src/lib.rs and reachable public modules/re-exports", ["published Rust library consumers"], "supported public library surface", [], ["cargo test --doc", "cargo test --lib"], "Deprecate and migrate before removal."),
    "mcp-production": ("src/mcp/server and tool metadata tests", ["registered MCP clients"], "supported MCP server", ["docs/specs/GH981/PRODUCT.md"], ["cargo test mcp::server::tests::tool_metadata"], "Preserve tool names and legacy wire contracts."),
    "rest-api": ("src/api/server.rs and src/api/handlers", ["authenticated loopback API clients"], "registered by build_router", [], ["cargo test --test api_public"], "Restore the previous router/handler pair."),
    "cli-production": ("src/cli/types.rs and src/cli/dispatch.rs", ["operators and host integrations"], "compiled in the default binary", [], ["cargo test cli::"], "Restore the prior parser/dispatch behavior."),
    "sessionstart-context-bundle": ("src/context and src/context_bundle", ["supported SessionStart host adapters"], "default bundle render mode", ["docs/specs/GH932/PRODUCT.md"], ["cargo test context_bundle"], "Set REMEM_CONTEXT_BUNDLE_RENDER_MODE=legacy."),
    "mcp-context-bundle": ("src/mcp/server/context_tools.rs and src/context_bundle", ["explicit MCP context_bundle callers"], "opt-in; not required by SessionStart", ["docs/specs/GH932/PRODUCT.md"], ["cargo test mcp::server::tests::context_bundle"], "Remove optional MCP registration; retain SessionStart."),
    "rust-context-bundle": ("src/context_bundle", ["explicit Rust context_bundle callers"], "opt-in; default path has a separate production adapter", ["docs/specs/GH932/TECH.md"], ["cargo test context_bundle"], "Remove exports; retain the production compiler."),
    "currenttruth-v1": ("src/truth and src/context_bundle/current_truth.rs", ["default Context Bundle and SessionStart projection"], "default Core projection", ["docs/specs/GH933/PRODUCT.md"], ["cargo test truth::"], "Use the broader old-path rollback."),
    "doctor-truth": ("src/doctor/truth.rs and src/truth", ["remem doctor truth operators"], "explicit read-only diagnostic", ["docs/specs/GH933/TECH.md"], ["cargo test doctor::truth"], "Restore the previous report format."),
    "retrieval-router-plan": ("src/retrieval_router and src/cli", ["explicit context-plan callers"], "opt-in plan compiler", ["docs/specs/GH934/PRODUCT.md"], ["cargo test retrieval_router"], "Remove context-plan; retain ordinary retrieval."),
    "rust-retrieval-router": ("src/retrieval_router", ["explicit Rust retrieval_router callers"], "opt-in; not a default executor", ["docs/specs/GH934/TECH.md"], ["cargo test retrieval_router"], "Remove exports; retain ordinary retrieval."),
    "routed-search-parameters": ("src/mcp/server/search_routing.rs and src/retrieval_router", ["search callers supplying routing parameters"], "opt-in; legacy search has a production entry", ["docs/specs/GH934/PRODUCT.md"], ["cargo test mcp::server::tests::search"], "Ignore/remove routing parameters."),
    "graph-edges": ("src/retrieval/graph and src/memory/graph_contract.rs", ["weighted production retrieval"], "default graph weight is non-zero", [], ["cargo run -- eval-gates --json-out /tmp/remem-eval-gates.json"], "Set graph weight to zero."),
    "entity-bfs": ("src/retrieval/entity and src/eval/graph_decision.rs", ["explicit graph-decision evaluation"], "opt-in informational arm", [], ["cargo run -- eval-graph-decision --help"], "Remove the diagnostic arm."),
    "local-onnx": ("src/retrieval/embedding and Cargo feature local-onnx", ["default builds with verified local artifacts"], "default Cargo feature", ["docs/specs/local-semantic-embedding/PRODUCT.md"], ["cargo test retrieval::embedding"], "Select another provider or disable the feature."),
    "deterministic-eval": ("src/eval and src/eval/gates.rs", ["CI/preflight and explicit eval CLI"], "eval is a default feature; commands are explicit", [], ["cargo run -- eval-gates --json-out /tmp/remem-eval-gates.json"], "Restore prior thresholds and baselines."),
    "coding-public-benchmarks": ("src/eval/coding_bench and eval/public", ["explicit benchmark runners/reporters"], "opt-in and non-claim-bearing until gates pass", ["docs/specs/GH931/PRODUCT.md", "docs/specs/public-memory-benchmark/PRODUCT.md"], ["cargo run -- eval-coding-bench --help"], "Stop new reports; retain immutable evidence."),
    "legacy-pending": ("src/db/pending/admin and worker idle bridge", ["idle-only residual drain", "explicit pending admin commands"], "bounded recovery; no new normal writer", ["docs/specs/legacy-observation-retirement/PRODUCT.md"], ["cargo test pending_recovery"], "Disable idle drain; retain diagnostics."),
    "legacy-events": ("src/db/capture.rs and legacy readers", ["transactional compatibility projection", "legacy readers"], "compatibility projection; not a source of truth", ["docs/specs/legacy-observation-retirement/TECH.md"], ["cargo test db::capture"], "Retain projection until removal prerequisites pass."),
    "historical-summary": ("src/db/failure_lifecycle and historical Summary readers", ["explicit diagnostics/history readers"], "new Summary dispatch is rejected", ["docs/specs/failure-lifecycle/PRODUCT.md"], ["cargo test failure_lifecycle"], "Retain read-only history until migration evidence."),
    "currenttruth-v2": ("docs/specs/GH933", ["none; planning only"], "unimplemented breaking cutover", ["docs/specs/GH933/PRODUCT.md", "docs/specs/GH933/TECH.md"], ["none until implementation lands"], "Keep CurrentTruth v1 active."),
    "cross-host-harness": ("eval/cross-host and docs/specs/GH935", ["explicit offline validation/dry-run commands"], "offline infrastructure; no live execution claim", ["docs/specs/GH935/PRODUCT.md", "docs/specs/GH935/TECH.md"], ["python3 eval/cross-host/scripts/test_run_dry.py"], "Retain schemas/tasks; disable optional dry runner."),
    "cross-host-completion": ("docs/specs/GH935", ["none; no live matrix"], "unimplemented beyond offline harness", ["docs/specs/GH935/PRODUCT.md", "docs/specs/GH935/TECH.md"], ["none until official runs exist"], "Continue infrastructure_only_no_runs."),
}


def lifecycle_row(kind: str, entry: str) -> str:
    if kind == "rust_export":
        overrides = {
            "remem::context_bundle": "rust-context-bundle",
            "remem::retrieval_router": "rust-retrieval-router",
            "remem::retrieval::entity": "entity-bfs",
            "remem::eval::graph_decision": "entity-bfs",
            "remem::eval::coding_bench": "coding-public-benchmarks",
        }
        for prefix, row_name in overrides.items():
            if entry == prefix or entry.startswith(prefix + "::"):
                return row_name
        return "rust-library"
    if kind == "mcp_tool":
        return "mcp-context-bundle" if entry == "context_bundle" else "mcp-production"
    if kind == "rest_route":
        return "rest-api"
    if kind == "default_feature":
        return "local-onnx" if entry == "local-onnx" else "deterministic-eval"
    if entry == "remem context-plan":
        return "retrieval-router-plan"
    if entry == "remem doctor truth":
        return "doctor-truth"
    if entry == "remem eval-graph-decision":
        return "entity-bfs"
    if entry == "remem eval-coding-bench" or entry == "remem bench" or entry.startswith("remem bench "):
        return "coding-public-benchmarks"
    if entry == "remem pending" or entry.startswith("remem pending "):
        return "legacy-pending"
    if entry == "remem eval" or entry.startswith("remem eval-"):
        return "deterministic-eval"
    return "cli-production"


def lifecycle_record(record_id: str, kind: str, entry: str, row_name: str, **extra: object) -> dict[str, object]:
    owner, callers, state, specs, evaluations, rollback = ROW_DETAILS[row_name]
    status = ROW_STATUS[row_name]
    recovery = {"normal_writers": []} if status == "recovery-only" else {}
    return {
        "id": record_id,
        "surface_kind": kind,
        "inventory_row": row_name,
        "owner": owner,
        "status": status,
        "public_entry_points": [entry],
        "real_callers": callers,
        "default_state": state,
        "spec_refs": [*specs, "docs/specs/GH969/PRODUCT.md"],
        "eval_commands": evaluations,
        "compatibility": f"Governed by canonical GH969 inventory row {row_name}.",
        "rollback": rollback,
        "decision_due": "2026-11-30" if status in {"experimental", "deprecated", "spec-only", "staged"} else None,
        **recovery,
        **extra,
    }


def _offline_inventory(root: Path, roots: list[str]) -> dict[str, object]:
    files = sorted(
        path.relative_to(root).as_posix()
        for declared_root in roots
        for path in (root / declared_root).rglob("*")
        if path.is_file()
    )
    return {
        "roots": roots,
        "executables": [path for path in files if "/scripts/" in path and path.endswith(".py")],
        "schemas": [path for path in files if "/schemas/" in path and path.endswith(".json")],
        "fixtures": [path for path in files if "/examples/" in path],
        "data": [path for path in files if "/tasks/" in path or path.endswith("benchmark-charter.json")],
        "documents": [path for path in files if not any(part in path for part in ("/scripts/", "/schemas/", "/examples/", "/tasks/")) and not path.endswith("benchmark-charter.json")],
        "checked_command": "python3 eval/cross-host/scripts/test_run_dry.py",
    }


def build_manifest(root: Path, *, doc_root: Path | None = None) -> dict[str, object]:
    records = [
        lifecycle_record(f"{kind}:{entry}", kind, entry, lifecycle_row(kind, entry))
        for kind, entries in discover_all(root, doc_root=doc_root).items()
        for entry in sorted(entries)
    ]
    specials = [
        ("runtime:sessionstart-context-bundle", "runtime_component", "src/context_bundle", "sessionstart-context-bundle"),
        ("runtime:currenttruth-v1", "runtime_component", "src/truth", "currenttruth-v1"),
        ("mcp-parameter:search-routing", "mcp_parameter", "src/mcp/server/search_routing.rs", "routed-search-parameters"),
        ("runtime:graph-edges", "runtime_component", "src/retrieval/graph", "graph-edges"),
        ("offline:coding-public-benchmarks", "runtime_component", "eval/public", "coding-public-benchmarks"),
        ("runtime:legacy-pending-idle-bridge", "recovery_path", "src/db/pending/admin", "legacy-pending"),
        ("runtime:legacy-events-projection", "compatibility_path", "src/db/capture.rs", "legacy-events"),
        ("runtime:historical-summary", "recovery_path", "src/db/failure_lifecycle.rs", "historical-summary"),
        ("spec:currenttruth-v2", "spec_contract", "docs/specs/GH933", "currenttruth-v2"),
        ("spec:cross-host-completion", "spec_contract", "docs/specs/GH935", "cross-host-completion"),
    ]
    for record_id, kind, entry, row_name in specials:
        recovery = {"normal_writers": [], "caller_paths": [entry]} if ROW_STATUS[row_name] == "recovery-only" else {}
        records.append(lifecycle_record(record_id, kind, entry, row_name, **recovery))
    roots = ["eval/cross-host", "docs/specs/GH935"]
    records.append(lifecycle_record("offline:cross-host-harness", "offline_harness", roots[0], "cross-host-harness", artifacts=_offline_inventory(root, roots)))
    return {
        "schema_version": 1,
        "canonical_contract": "docs/specs/GH969/PRODUCT.md#canonical-surface-inventory",
        "generated_by": "python3 scripts/ci/surface_lifecycle_discovery.py --write-manifest",
        "records": sorted(records, key=lambda item: str(item["id"])),
    }


def render_manifest(manifest: dict[str, object]) -> str:
    lines = ["{"]
    for key in ("schema_version", "canonical_contract", "generated_by"):
        lines.append(f"  {json.dumps(key)}: {json.dumps(manifest[key])},")
    records = manifest["records"]
    assert isinstance(records, list)
    lines.append('  "records": [')
    for index, record in enumerate(records):
        suffix = "," if index + 1 < len(records) else ""
        lines.append("    " + json.dumps(record, separators=(",", ":")) + suffix)
    lines.extend(["  ]", "}"])
    return "\n".join(lines) + "\n"


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--write-manifest", action="store_true")
    parser.add_argument(
        "--output",
        type=Path,
        default=ROOT / "docs/specs/GH969/surface-manifest.json",
    )
    args = parser.parse_args()
    if not args.write_manifest:
        parser.error("pass --write-manifest for an explicit reviewed regeneration")
    manifest = build_manifest(ROOT)
    args.output.write_text(render_manifest(manifest), encoding="utf-8")
    records = manifest["records"]
    assert isinstance(records, list)
    print(f"wrote {len(records)} lifecycle records to {args.output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
