#!/usr/bin/env python3
"""Discover callable remem surfaces from their real export/registration roots."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
from pathlib import Path

from surface_lifecycle_evidence import (
    EXPERIMENTAL_CALLER_SYMBOLS,
    PRODUCTION_DEFAULT_GUARDS,
    build_caller_guard,
    build_default_guard,
    discover_product_rows,
    discover_search_parameters,
    discover_target_gated_exports,
    expanded_default_features,
    offline_categories,
)
from surface_lifecycle_rust import discover_rust_exports
from surface_lifecycle_mcp import discover_mcp_legacy_shapes, discover_mcp_schema_fingerprints
from surface_lifecycle_release import verified_release_baseline
from surface_lifecycle_rest import (
    discover_rest_middleware_fingerprint,
    discover_rest_schema_fingerprints,
    reject_unsupported_method_routes,
    reject_unsupported_router_methods,
)


HTTP_METHODS = ("delete", "get", "head", "options", "patch", "post", "put", "trace")
ROOT = Path(__file__).resolve().parents[2]
DISCOVERED_SURFACE_KINDS = {
    "rust_export", "rust_target_export", "mcp_tool", "mcp_parameter",
    "rest_route", "cli_command", "default_feature",
}


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
        elif char == '"' or (char == "'" and re.match(r"'(?:\\(?:x[0-9A-Fa-f]{2}|u\{[0-9A-Fa-f_]+\}|.)|[^\\'])'", text[index:])):
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
        elif char == '"' or (char == "'" and re.match(r"'(?:\\(?:x[0-9A-Fa-f]{2}|u\{[0-9A-Fa-f_]+\}|.)|[^\\'])'", text[index:])):
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


def _method_calls(text: str, method_name: str) -> list[list[str]]:
    calls: list[list[str]] = []
    cursor = 0
    while True:
        opening = text.find(f".{method_name}(", cursor)
        if opening < 0:
            break
        opening += len(method_name) + 1
        closing = _matching(text, opening, "(", ")")
        if closing is None:
            raise RuntimeError(f"unclosed .{method_name} call")
        calls.append(_top_level_parts(text[opening + 1 : closing]))
        cursor = closing + 1
    return calls


def _rust_functions(root: Path) -> dict[str, tuple[Path, str]]:
    functions: dict[str, tuple[Path, str]] = {}
    for path in sorted((root / "src/api").rglob("*.rs")):
        if "tests" in path.parts or path.name == "tests.rs":
            continue
        raw_text = path.read_text(encoding="utf-8")
        if not any(marker in raw_text for marker in ("Router::", "Router<", ".route(", ".nest(", ".merge(")):
            continue
        text = _mask_comments(raw_text)
        for match in re.finditer(r"\bfn\s+([A-Za-z_][A-Za-z0-9_]*)\s*\(", text):
            arguments_end = _matching(text, match.end() - 1, "(", ")")
            if arguments_end is None:
                raise RuntimeError(f"unclosed function arguments in {path}")
            body_start = text.find("{", arguments_end)
            semicolon = text.find(";", arguments_end)
            if body_start < 0 or 0 <= semicolon < body_start:
                continue
            body_end = _matching(text, body_start, "{", "}")
            if body_end is None:
                raise RuntimeError(f"unclosed function body in {path}")
            name = match.group(1)
            if name in functions:
                raise RuntimeError(f"ambiguous API router helper function {name!r}")
            functions[name] = (path, text[body_start + 1 : body_end])
    return functions


def _router_helper(argument: str, *, path: Path, method: str) -> str:
    helper = re.fullmatch(
        r"\s*(?:[A-Za-z_][A-Za-z0-9_]*::)*(?P<name>[A-Za-z_][A-Za-z0-9_]*)\s*\([^)]*\)\s*",
        argument,
    )
    if not helper:
        raise RuntimeError(
            f".{method} in {path} must reference a named router helper so lifecycle discovery cannot silently drop routes: {argument.strip()}"
        )
    return helper.group("name")


def discover_rest_routes(root: Path, *, schema_fingerprints: dict[str, str] | None = None) -> set[str]:
    functions = _rust_functions(root)
    if "build_router" not in functions:
        raise RuntimeError("cannot resolve API build_router")
    routes: set[str] = set()
    schemas = discover_rest_schema_fingerprints(root) if schema_fingerprints is None else schema_fingerprints
    router_sources: list[tuple[str, str]] = []

    def visit(name: str, prefix: str, ancestors: tuple[str, ...]) -> None:
        if name in ancestors:
            raise RuntimeError(f"recursive Axum router graph: {' -> '.join((*ancestors, name))}")
        if name not in functions:
            raise RuntimeError(f"Axum router helper {name!r} is referenced but not discovered")
        path, body = functions[name]
        router_sources.append((path.relative_to(root).as_posix(), body))
        reject_unsupported_router_methods(body, path)
        for local in re.finditer(r"\blet\s+(?P<var>[A-Za-z_][A-Za-z0-9_]*)[^=;]*=\s*(?P<helper>[A-Za-z_][A-Za-z0-9_]*)\s*\([^;]*\)\s*;", body):
            if local.group("helper") in functions and re.search(rf"\b{re.escape(local.group('var'))}\s*\.\s*(?:route|merge|nest|route_layer|layer)\s*\(", body[local.end() :]):
                visit(local.group("helper"), prefix, (*ancestors, name))
        for arguments in _method_calls(body, "route"):
            if len(arguments) < 2:
                raise RuntimeError(f"route call has fewer than two arguments in {path}")
            reject_unsupported_method_routes(arguments[1], path)
            path_match = re.fullmatch(r'\s*"([^\"]+)"\s*', arguments[0])
            if not path_match:
                raise RuntimeError(f"route path is not a literal in {path}: {arguments[0].strip()}")
            method_handlers = {
                method.upper(): match.group(1)
                for method in HTTP_METHODS
                if (match := re.search(rf"\b{method}\s*\(\s*(?:[A-Za-z_][A-Za-z0-9_]*::)*([A-Za-z_][A-Za-z0-9_]*)\s*\)", arguments[1]))
            }
            if "GET" in method_handlers:
                method_handlers.setdefault("HEAD", method_handlers["GET"])
            if not method_handlers:
                raise RuntimeError(f"route has no recognized HTTP method in {path}: {arguments[1].strip()}")
            route_path = prefix.rstrip("/") + "/" + path_match.group(1).lstrip("/")
            for method, handler in method_handlers.items():
                if schemas and handler not in schemas:
                    raise RuntimeError(f"REST route {method} {route_path} references unclassified handler {handler!r}")
                suffix = f"@sha256={schemas[handler]}" if schemas else ""
                routes.add(f"{method} {route_path}{suffix}")
        for arguments in _method_calls(body, "merge"):
            if len(arguments) != 1:
                raise RuntimeError(f"merge call must have one router argument in {path}")
            visit(_router_helper(arguments[0], path=path, method="merge"), prefix, (*ancestors, name))
        for arguments in _method_calls(body, "nest"):
            if len(arguments) != 2:
                raise RuntimeError(f"nest call must have a prefix and router argument in {path}")
            path_match = re.fullmatch(r'\s*"([^\"]+)"\s*', arguments[0])
            if not path_match:
                raise RuntimeError(f"nest prefix is not a literal in {path}: {arguments[0].strip()}")
            nested_prefix = prefix.rstrip("/") + "/" + path_match.group(1).strip("/")
            visit(_router_helper(arguments[1], path=path, method="nest"), nested_prefix, (*ancestors, name))

    visit("build_router", "", ())
    middleware = discover_rest_middleware_fingerprint(root, router_sources) if schemas else None
    if middleware:
        routes = {f"{route}@router-sha256={middleware}" for route in routes}
    if not routes:
        raise RuntimeError("no REST routes discovered from build_router")
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
        elif char == '"' or (char == "'" and re.match(r"'(?:\\(?:x[0-9A-Fa-f]{2}|u\{[0-9A-Fa-f_]+\}|.)|[^\\'])'", text[index:])):
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


def _cfg_enabled(expression: str, features: set[str]) -> bool:
    value = expression.strip()
    feature = re.fullmatch(r'feature\s*=\s*"([^"]+)"', value)
    if feature:
        return feature.group(1) in features
    call = re.fullmatch(r"(all|any|not)\s*\((.*)\)", value, re.S)
    if call:
        values = [_cfg_enabled(part, features) for part in _top_level_parts(call.group(2))]
        if call.group(1) == "all":
            return all(values)
        if call.group(1) == "any":
            return any(values)
        if len(values) != 1:
            raise RuntimeError(f"cfg(not(...)) requires one predicate: {expression}")
        return not values[0]
    raise RuntimeError(f"unsupported CLI cfg predicate {expression!r}; discovery must fail closed")


def _leading_attributes(raw_variant: str) -> list[str]:
    value = _mask_comments(raw_variant).lstrip()
    attributes: list[str] = []
    while value.startswith("#"):
        opening = value.find("[")
        closing = _matching(value, opening, "[", "]") if opening >= 0 else None
        if closing is None:
            raise RuntimeError(f"unclosed Clap variant attribute: {raw_variant.strip()}")
        attributes.append(value[opening + 1 : closing].strip())
        value = value[closing + 1 :].lstrip()
    return attributes


def _command_names(variant: str, attributes: list[str]) -> tuple[str, ...]:
    command_attributes = [item for item in attributes if re.match(r"command\s*\(", item)]
    combined = "\n".join(command_attributes)
    override = re.search(r'\bname\s*=\s*"([^"]+)"', combined)
    names = [override.group(1) if override else _kebab_case(variant)]
    names.extend(
        match.group(1)
        for match in re.finditer(r'\b(?:visible_alias|alias)\s*=\s*"([^"]+)"', combined)
    )
    for match in re.finditer(r"\b(?:visible_aliases|aliases)\s*=\s*\[([^]]*)\]", combined, re.S):
        names.extend(re.findall(r'"([^"]+)"', match.group(1)))
    return tuple(dict.fromkeys(names))


def _clap_args_contracts(root: Path) -> dict[str, str]:
    declarations: dict[str, str] = {}
    pattern = re.compile(
        r"#\s*\[\s*derive\s*\([^]]*\b(?:Args|ValueEnum)\b[^]]*\)\s*\]"
        r"(?:(?:\s|#\s*\[[^]]*\])*)(?:pub(?:\s*\([^)]*\))?\s+)?(?:struct|enum)\s+"
        r"(?P<name>[A-Za-z_][A-Za-z0-9_]*)",
        re.S,
    )
    for path in sorted((root / "src/cli").glob("*.rs")):
        text = path.read_text(encoding="utf-8")
        for match in pattern.finditer(text):
            cursor = match.end()
            while cursor < len(text) and text[cursor].isspace():
                cursor += 1
            if cursor >= len(text) or text[cursor] not in "{(":
                raise RuntimeError(f"unsupported Clap Args struct {match.group('name')} in {path}")
            right = "}" if text[cursor] == "{" else ")"
            end = _matching(text, cursor, text[cursor], right)
            name = match.group("name")
            if name in declarations:
                raise RuntimeError(f"ambiguous Clap Args struct basename {name!r}")
            declarations[name] = text[match.start() : end + 1]
    cache: dict[str, str] = {}

    def resolve(name: str, ancestors: tuple[str, ...]) -> str:
        if name in ancestors:
            raise RuntimeError(f"recursive Clap Args graph: {' -> '.join((*ancestors, name))}")
        if name in cache:
            return cache[name]
        raw = declarations[name]
        nested = sorted(other for other in declarations if other != name and re.search(rf"\b{re.escape(other)}\b", raw))
        evidence = [re.sub(r"\s+", " ", _mask_comments(raw)).strip()]
        evidence.extend(resolve(other, (*ancestors, name)) for other in nested)
        cache[name] = hashlib.sha256("\n".join(evidence).encode()).hexdigest()
        return cache[name]

    return {name: resolve(name, ()) for name in declarations}


def _subcommand_enums(root: Path, features: set[str]) -> dict[str, list[tuple[tuple[str, ...], str | None, str]]]:
    enums: dict[str, list[tuple[tuple[str, ...], str | None, str]]] = {}
    args_contracts = _clap_args_contracts(root)
    for path in sorted((root / "src/cli").glob("*.rs")):
        text = path.read_text(encoding="utf-8")
        enum_re = re.compile(
            r"(?P<attributes>(?:#\s*\[(?!\s*derive\s*\([^]]*\bSubcommand\b)[^]]*\]\s*)*)"
            r"#\s*\[\s*derive\s*\([^]]*\bSubcommand\b[^]]*\)\s*\]"
            r"(?:(?:\s|#\s*\[[^]]*\])*)"
            r"(?:pub(?:\s*\([^)]*\))?\s+)?enum\s+(?P<name>[A-Za-z_][A-Za-z0-9_]*)\s*\{",
            re.S,
        )
        for match in enum_re.finditer(text):
            enum_cfgs = [
                cfg_match.group(1)
                for attribute in _leading_attributes(match.group("attributes"))
                if (cfg_match := re.fullmatch(r"cfg\s*\((.*)\)", attribute, re.S))
            ]
            if not all(_cfg_enabled(cfg, features) for cfg in enum_cfgs):
                continue
            opening = match.end() - 1
            closing = _matching(text, opening, "{", "}")
            if closing is None:
                raise RuntimeError(f"unclosed Subcommand enum {match.group('name')} in {path}")
            variants: list[tuple[tuple[str, ...], str | None, str]] = []
            for raw_variant in _top_level_parts(text[opening + 1 : closing]):
                if not raw_variant.strip():
                    continue
                attributes = _leading_attributes(raw_variant)
                cfgs = [
                    cfg_match.group(1)
                    for attribute in attributes
                    if (cfg_match := re.fullmatch(r"cfg\s*\((.*)\)", attribute, re.S))
                ]
                if not all(_cfg_enabled(cfg, features) for cfg in cfgs):
                    continue
                visible = _strip_leading_attributes(_mask_comments(raw_variant))
                variant_match = re.match(r"([A-Za-z_][A-Za-z0-9_]*)", visible)
                if not variant_match:
                    continue
                variant = variant_match.group(1)
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
                contract = re.sub(r"\s+", " ", _mask_comments(raw_variant)).strip()
                contract += "".join(
                    f"\n{name}:{digest}" for name, digest in sorted(args_contracts.items())
                    if re.search(rf"\b{re.escape(name)}\b", raw_variant)
                )
                variants.append((_command_names(variant, attributes), child, hashlib.sha256(contract.encode()).hexdigest()))
            enum_name = match.group("name")
            if enum_name in enums:
                raise RuntimeError(f"ambiguous Clap Subcommand enum basename {enum_name!r}")
            enums[enum_name] = variants
    return enums


def discover_cli_commands(root: Path) -> set[str]:
    enums = _subcommand_enums(root, discover_default_features(root))
    if "Commands" not in enums:
        raise RuntimeError("cannot resolve root Clap Commands enum under src/cli")
    commands: set[str] = set()

    def visit(enum_name: str, prefix: tuple[str, ...], ancestors: tuple[str, ...]) -> None:
        if enum_name in ancestors:
            raise RuntimeError(f"recursive Clap subcommand graph: {' -> '.join((*ancestors, enum_name))}")
        if enum_name not in enums:
            raise RuntimeError(f"Clap subcommand enum {enum_name} is referenced but not discovered")
        commands.add("remem " + " ".join((*prefix, "help")))
        for command_names, child, _ in enums[enum_name]:
            for command in command_names:
                path = (*prefix, command)
                commands.add("remem " + " ".join(path))
                if child:
                    visit(child, path, (*ancestors, enum_name))

    visit("Commands", (), ())
    return commands


def discover_cli_contracts(root: Path) -> set[str]:
    enums = _subcommand_enums(root, discover_default_features(root))
    if "Commands" not in enums:
        raise RuntimeError("cannot resolve root Clap Commands enum under src/cli")
    commands: set[str] = set()

    def visit(enum_name: str, prefix: tuple[str, ...], ancestors: tuple[str, ...], parents: tuple[str, ...]) -> None:
        if enum_name in ancestors:
            raise RuntimeError(f"recursive Clap subcommand graph: {' -> '.join((*ancestors, enum_name))}")
        if enum_name not in enums:
            raise RuntimeError(f"Clap subcommand enum {enum_name} is referenced but not discovered")
        variants = enums[enum_name]
        help_digest = hashlib.sha256("\n".join(sorted((*parents, *(item[2] for item in variants)))).encode()).hexdigest()
        commands.add("remem " + " ".join((*prefix, "help")) + f"@sha256={help_digest}")
        for command_names, child, digest in variants:
            chain = (*parents, digest)
            fingerprint = hashlib.sha256("\n".join(chain).encode()).hexdigest()
            for command in command_names:
                path = (*prefix, command)
                commands.add("remem " + " ".join(path) + f"@sha256={fingerprint}")
                if child:
                    visit(child, path, (*ancestors, enum_name), chain)

    visit("Commands", (), (), ())
    return commands


def discover_default_features(root: Path) -> set[str]:
    return expanded_default_features(root)


def discover_all(root: Path, *, doc_root: Path | None = None, mcp_fingerprints: dict[str, str] | None = None, rest_fingerprints: dict[str, str] | None = None) -> dict[str, set[str]]:
    mcp_tools, _ = discover_mcp_tools(root)
    schemas = mcp_fingerprints or discover_mcp_schema_fingerprints(root)
    if set(schemas) != mcp_tools:
        raise RuntimeError("served MCP schema set contradicts the contract registry")
    legacy_shapes = discover_mcp_legacy_shapes(root)
    if set(legacy_shapes) != mcp_tools:
        raise RuntimeError("MCP legacy response shapes contradict the contract registry")
    mcp_contracts = {
        name: f"{schemas[name]}@legacy-sha256={hashlib.sha256(legacy_shapes[name].encode()).hexdigest()}"
        for name in mcp_tools
    }
    return {
        "rust_export": discover_rust_exports(root, doc_root=doc_root),
        "rust_target_export": discover_target_gated_exports(root),
        "mcp_tool": {f"{name}@sha256={mcp_contracts[name]}" for name in mcp_tools},
        "mcp_parameter": discover_search_parameters(root),
        "rest_route": discover_rest_routes(root, schema_fingerprints=rest_fingerprints),
        "cli_command": discover_cli_contracts(root),
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
    entry = entry.split("@sha256=", 1)[0]
    if kind == "rust_target_export":
        source_overrides = {
            "src/context_bundle": "rust-context-bundle",
            "src/retrieval_router": "rust-retrieval-router",
            "src/retrieval/entity": "entity-bfs",
            "src/eval/graph_decision": "entity-bfs",
            "src/eval/coding_bench": "coding-public-benchmarks",
        }
        return next((row for prefix, row in source_overrides.items() if entry.startswith(prefix)), "rust-library")
    if kind in {"rust_export", "rust_target_export"}:
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
    if kind == "mcp_parameter":
        routed = {"task_intent", "role", "risk", "token_budget", "include_superseded"}
        return "routed-search-parameters" if entry.rsplit(".", 1)[-1] in routed else "mcp-production"
    if kind == "rest_route":
        return "rest-api"
    if kind == "default_feature":
        rows = {"local-onnx": "local-onnx", "eval": "deterministic-eval"}
        if entry not in rows:
            raise RuntimeError(f"default Cargo feature {entry!r} lacks an explicit lifecycle row")
        return rows[entry]
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


def lifecycle_record(record_id: str, kind: str, entry: str, row_name: str, *, canonical_rows: dict[str, dict[str, str]] | None = None, published: bool = True, **extra: object) -> dict[str, object]:
    owner, callers, state, specs, evaluations, rollback = ROW_DETAILS[row_name]
    status = ROW_STATUS[row_name]
    canonical = (canonical_rows or {}).get(row_name)
    if canonical:
        owner, state = canonical["owner"], canonical["real_caller_default"]
    if status == "production" and not published:
        status = "staged"
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
        "canonical_entry": canonical["entry"] if canonical else row_name,
        "evidence": canonical["evidence"] if canonical else "fixture evidence",
        "compatibility": canonical["compatibility"] if canonical else f"Governed by canonical GH969 inventory row {row_name}.",
        "next_decision": canonical["next_decision"] if canonical else "fixture decision",
        "rollback": rollback,
        "decision_due": "2026-11-30" if status in {"experimental", "deprecated", "spec-only", "staged"} else None,
        **recovery,
        **extra,
    }


def _offline_inventory(root: Path, roots: list[str]) -> dict[str, object]:
    return {
        "roots": roots,
        **offline_categories(root, roots),
        "checked_command": "python3 eval/cross-host/scripts/test_run_dry.py",
    }


def build_manifest(root: Path, *, doc_root: Path | None = None, published_surfaces: dict[str, set[str]] | None = None, published_release: str = "v0.6.82", retired_surfaces: dict[str, set[str]] | None = None) -> dict[str, object]:
    discovered = discover_all(root, doc_root=doc_root)
    canonical = discover_product_rows(root)
    published = published_surfaces if published_surfaces is not None else {kind: set(entries) for kind, entries in discovered.items()}
    records = [
        lifecycle_record(f"{kind}:{entry}", kind, entry, lifecycle_row(kind, entry), canonical_rows=canonical, published=entry in published.get(kind, set()))
        for kind, entries in discovered.items()
        for entry in sorted(entries)
    ]
    specials = [
        ("runtime:sessionstart-context-bundle", "runtime_component", "src/context_bundle", "sessionstart-context-bundle"),
        ("runtime:currenttruth-v1", "runtime_component", "src/truth", "currenttruth-v1"),
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
        if record_id == "runtime:legacy-pending-idle-bridge":
            recovery["writer_guard"] = {"mode": "sql_table_insert", "target": "pending_observations"}
        if record_id == "runtime:historical-summary":
            recovery["writer_guard"] = {"mode": "summary_job_enqueue", "target": "summary"}
        records.append(lifecycle_record(record_id, kind, entry, row_name, canonical_rows=canonical, **recovery))
    for row_name, symbols in EXPERIMENTAL_CALLER_SYMBOLS.items():
        guarded = next(record for record in records if record["inventory_row"] == row_name)
        guarded["caller_guard"] = build_caller_guard(root, symbols)
    for row_name, mode in PRODUCTION_DEFAULT_GUARDS.items():
        guarded = next(record for record in records if record["inventory_row"] == row_name)
        guarded["default_guard"] = build_default_guard(root, mode)
    roots = ["eval/cross-host", "docs/specs/GH935"]
    records.append(lifecycle_record("offline:cross-host-harness", "offline_harness", roots[0], "cross-host-harness", canonical_rows=canonical, artifacts=_offline_inventory(root, roots)))
    return {
        "schema_version": 2,
        "canonical_contract": "docs/specs/GH969/PRODUCT.md#canonical-surface-inventory",
        "generated_by": "python3 scripts/ci/surface_lifecycle_discovery.py --write-manifest",
        "published_release": published_release,
        "published_surfaces": {kind: sorted(entries) for kind, entries in published.items()},
        "retired_surfaces": {kind: sorted(entries) for kind, entries in (retired_surfaces or {kind: set() for kind in DISCOVERED_SURFACE_KINDS}).items()},
        "records": sorted(records, key=lambda item: str(item["id"])),
    }
def render_manifest(manifest: dict[str, object]) -> str:
    lines = ["{"]
    for key in ("schema_version", "canonical_contract", "generated_by", "published_release", "published_surfaces", "retired_surfaces"):
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
    parser.add_argument("--promote-published", metavar="RELEASE")
    parser.add_argument(
        "--output",
        type=Path,
        default=ROOT / "docs/specs/GH969/surface-manifest.json",
    )
    args = parser.parse_args()
    if not args.write_manifest:
        parser.error("pass --write-manifest for an explicit reviewed regeneration")
    previous = json.loads(args.output.read_text(encoding="utf-8")) if args.output.is_file() else {}
    baseline = previous.get("published_surfaces")
    if not isinstance(baseline, dict):
        parser.error("existing manifest must contain published_surfaces; bootstrap is not a normal regeneration path")
    published = {kind: set(entries) for kind, entries in baseline.items()}
    retirements = previous.get("retired_surfaces", {kind: [] for kind in DISCOVERED_SURFACE_KINDS})
    if not isinstance(retirements, dict): parser.error("retired_surfaces must be a kind-to-entry-list object")
    release = str(previous.get("published_release", "v0.6.82"))
    if args.promote_published:
        release = args.promote_published
        published = verified_release_baseline(release, DISCOVERED_SURFACE_KINDS)
    manifest = build_manifest(ROOT, published_surfaces=published, published_release=release, retired_surfaces={kind: set(entries) for kind, entries in retirements.items()})
    args.output.write_text(render_manifest(manifest), encoding="utf-8")
    records = manifest["records"]
    assert isinstance(records, list)
    print(f"wrote {len(records)} lifecycle records to {args.output}")
    return 0


if __name__ == "__main__": raise SystemExit(main())
