#!/usr/bin/env python3
"""Fingerprint REST wire declarations and reject untracked Axum registrations."""

from __future__ import annotations

import hashlib
import re
import tempfile
from pathlib import Path

from surface_lifecycle_evidence import mask_cfg_test_blocks, mask_rust_comments


UNSUPPORTED_ROUTER_METHODS = (
    "route_service", "nest_service", "fallback", "fallback_service",
)
SUPPORTED_ROUTE_METHODS = {"delete", "get", "head", "options", "patch", "post", "put", "trace"}
UNSUPPORTED_METHOD_ROUTES = {"any", "any_service", "connect", "connect_service", "on", "on_service"}


def _matching(text: str, opening: int, left: str, right: str) -> int:
    depth = 0
    quote: str | None = None
    escaped = False
    for index in range(opening, len(text)):
        char = text[index]
        if quote:
            if escaped:
                escaped = False
            elif char == "\\":
                escaped = True
            elif char == quote:
                quote = None
        elif char == '"' or (char == "'" and re.match(r"'(?:\\(?:x[0-9A-Fa-f]{2}|u\{[0-9A-Fa-f_]+\}|.)|[^\\'])'", text[index:])):
            quote = char
        elif char == left:
            depth += 1
        elif char == right:
            depth -= 1
            if depth == 0:
                return index
    raise RuntimeError("unclosed REST schema declaration")


def _normalize(value: str) -> str:
    value = mask_rust_comments(value)
    return re.sub(r"\s+", " ", value).strip()


def _wire_declarations(text: str) -> list[str]:
    declarations: list[str] = []
    pattern = re.compile(
        r"(?P<attrs>(?:\s*#\s*\[[^]]*\])+)\s*"
        r"(?:pub(?:\s*\([^)]*\))?\s+)?(?P<kind>struct|enum)\s+"
        r"[A-Za-z_][A-Za-z0-9_]*(?:\s*<[^>{;]*>)?",
        re.S,
    )
    for match in pattern.finditer(text):
        if not re.search(r"\b(?:Serialize|Deserialize)\b", match.group("attrs")):
            continue
        cursor = match.end()
        while cursor < len(text) and text[cursor].isspace():
            cursor += 1
        if cursor == len(text):
            raise RuntimeError("unterminated REST wire declaration")
        if text[cursor] == "{":
            end = _matching(text, cursor, "{", "}") + 1
        elif text[cursor] == "(":
            end = _matching(text, cursor, "(", ")") + 1
            while end < len(text) and text[end].isspace():
                end += 1
            if end >= len(text) or text[end] != ";":
                raise RuntimeError("REST tuple wire declaration lacks semicolon")
            end += 1
        elif text[cursor] == ";":
            end = cursor + 1
        else:
            raise RuntimeError("unsupported REST wire declaration shape")
        declarations.append(_normalize(text[match.start() : end]))
    custom_serde = re.compile(
        r"\bimpl(?:\s*<[^>{}]*>)?\s+(?:serde::)?"
        r"(?:Serialize|Deserialize(?:\s*<[^>{}]*>)?)\s+for\s+[^{}]+\{",
        re.S,
    )
    for match in custom_serde.finditer(text):
        declarations.append(
            _normalize(text[match.start() : _matching(text, match.end() - 1, "{", "}") + 1])
        )
    for match in re.finditer(r"(?:serde_json::)?json!\s*([({])", text):
        left = match.group(1)
        right = ")" if left == "(" else "}"
        declarations.append(_normalize(text[match.start() : _matching(text, match.end() - 1, left, right) + 1]))
    return declarations


def _function_evidence(text: str, name: str) -> list[str]:
    evidence: list[str] = []
    pattern = re.compile(rf"\bfn\s+{re.escape(name)}\s*(?:<[^>{{}};]*>)?\s*\(")
    for match in pattern.finditer(text):
        arguments_end = _matching(text, match.end() - 1, "(", ")")
        body_start = text.find("{", arguments_end)
        semicolon = text.find(";", arguments_end)
        if body_start < 0 or 0 <= semicolon < body_start:
            continue
        evidence.append(_normalize(text[match.start() : _matching(text, body_start, "{", "}") + 1]))
    return evidence


def _serde_helper_evidence(
    declaration: str,
    sources: list[tuple[str, str]],
) -> list[str]:
    direct = set(re.findall(r"\b(?:serialize_with|deserialize_with)\s*=\s*\"([^\"]+)\"", declaration))
    modules = set(re.findall(r"\bwith\s*=\s*\"([^\"]+)\"", declaration))
    found: set[str] = set()
    for helper in direct:
        name = helper.rsplit("::", 1)[-1]
        matches = [f"{relative}:{item}" for relative, source in sources for item in _function_evidence(source, name)]
        if not matches:
            raise RuntimeError(f"cannot resolve serde helper implementation {helper!r}")
        found.update(matches)
    for helper in modules:
        name = helper.rsplit("::", 1)[-1]
        matches: list[str] = []
        for relative, source in sources:
            for module in re.finditer(rf"\bmod\s+{re.escape(name)}\s*\{{", source):
                end = _matching(source, module.end() - 1, "{", "}")
                matches.append(f"{relative}:{_normalize(source[module.start() : end + 1])}")
            path = Path(relative)
            if path.stem == name or (path.name == "mod.rs" and path.parent.name == name):
                for function in ("serialize", "deserialize"):
                    matches.extend(
                        f"{relative}:{item}" for item in _function_evidence(source, function)
                    )
        if not matches:
            raise RuntimeError(f"cannot resolve serde helper module {helper!r}")
        found.update(matches)
    return sorted(found)


def _response_behavior(body: str, source: str | None = None) -> str:
    evidence = set(re.findall(r"\bStatusCode::[A-Z][A-Z0-9_]*\b", body))
    evidence.update(
        f'{field}="{value}"'
        for field, value in re.findall(r'\b(code|error_code)\s*:\s*"([^"\\]*)"', body)
    )
    for match in re.finditer(r"\b(?P<name>[A-Za-z_][A-Za-z0-9_]*)\s*\(", body):
        if not any(marker in match.group("name").lower() for marker in ("error", "invalid", "response")):
            continue
        closing = _matching(body, match.end() - 1, "(", ")")
        evidence.add(_normalize(body[match.start() : closing + 1]))
        if source is not None:
            evidence.update(
                f"helper:{match.group('name')}:{_response_behavior(item)}"
                for item in _function_evidence(source, match.group("name"))
            )
    return "\n".join(sorted(evidence))


def _handler_signatures(text: str) -> dict[str, str]:
    signatures: dict[str, str] = {}
    for match in re.finditer(
        r"pub\s*\(in\s+crate::api\)\s+async\s+fn\s+(?P<name>handle_[A-Za-z0-9_]+)\s*\(", text,
    ):
        arguments_end = _matching(text, match.end() - 1, "(", ")")
        body_start = text.find("{", arguments_end)
        if body_start < 0:
            raise RuntimeError(f"REST handler {match.group('name')} has no body")
        body_end = _matching(text, body_start, "{", "}")
        signature = _normalize(text[match.start() : body_start])
        signatures[match.group("name")] = f"{signature}\nresponse:{_response_behavior(text[body_start + 1 : body_end], text)}"
    return signatures


def _production_sources(root: Path) -> list[tuple[str, str]]:
    sources: list[tuple[str, str]] = []
    for path in sorted((root / "src").rglob("*.rs")):
        if (
            "tests" in path.parts
            or path.name in {"tests.rs", "test_support.rs"}
            or path.stem.endswith("_tests")
        ):
            continue
        sources.append((
            path.relative_to(root).as_posix(),
            mask_cfg_test_blocks(path.read_text(encoding="utf-8")),
        ))
    return sources


def _wire_index(sources: list[tuple[str, str]]) -> dict[str, list[str]]:
    index: dict[str, list[str]] = {}
    for relative, source in sources:
        for declaration in _wire_declarations(source):
            named = re.search(
                r"\b(?:struct|enum)\s+([A-Za-z_][A-Za-z0-9_]*)|"
                r"\bimpl(?:\s*<[^>{}]*>)?\s+(?:serde::)?(?:Serialize|Deserialize(?:\s*<[^>{}]*>)?)"
                r"\s+for\s+([A-Za-z_][A-Za-z0-9_]*)",
                declaration,
            )
            if named:
                name = named.group(1) or named.group(2)
                evidence = [f"{relative}:{declaration}"]
                evidence.extend(_serde_helper_evidence(declaration, sources))
                index.setdefault(name, []).extend(evidence)
    return index


def _referenced_crate_paths(text: str) -> set[str]:
    paths = set(re.findall(r"\bcrate::((?:[A-Za-z_][A-Za-z0-9_]*::)+[A-Za-z_][A-Za-z0-9_]*)", text))
    for match in re.finditer(
        r"\buse\s+crate::(?P<prefix>(?:[A-Za-z_][A-Za-z0-9_]*::)*[A-Za-z_][A-Za-z0-9_]*)::"
        r"\{(?P<items>[^{};]+)\}\s*;",
        text,
    ):
        for raw_item in match.group("items").split(","):
            item = re.sub(r"\s+as\s+[A-Za-z_][A-Za-z0-9_]*\s*$", "", raw_item).strip()
            if item and item != "self":
                paths.add(f"{match.group('prefix')}::{item}")
    for match in re.finditer(
        r"\buse\s+crate::(?P<path>(?:[A-Za-z_][A-Za-z0-9_]*::)*[A-Za-z_][A-Za-z0-9_]*)"
        r"(?:\s+as\s+(?P<alias>[A-Za-z_][A-Za-z0-9_]*))?\s*;",
        text,
    ):
        path = match.group("path")
        paths.add(path)
        alias = match.group("alias") or path.rsplit("::", 1)[-1]
        paths.update(
            f"{path}::{called}"
            for called in re.findall(rf"\b{re.escape(alias)}::([A-Za-z_][A-Za-z0-9_]*)\s*\(", text)
        )
    return paths


def _external_wire_declarations(
    root: Path,
    text: str,
    wire_index: dict[str, list[str]],
) -> list[str]:
    reachable = {
        name for name in re.findall(r"\b[A-Z][A-Za-z0-9_]*\b", text)
        if name in wire_index
    }
    for referenced_path in _referenced_crate_paths(text):
        parts = referenced_path.split("::")
        module_root: Path | None = None
        for length in range(len(parts) - 1, 0, -1):
            relative = Path(*parts[:length])
            candidates = (root / "src" / relative / "mod.rs", root / "src" / f"{relative}.rs")
            module_root = next((candidate for candidate in candidates if candidate.is_file()), None)
            if module_root is not None:
                break
        if module_root is None or "api" in module_root.relative_to(root / "src").parts:
            continue
        item = parts[-1]
        if item in wire_index:
            reachable.add(item)
        source = mask_cfg_test_blocks(module_root.read_text(encoding="utf-8"))
        function = re.search(rf"\bfn\s+{re.escape(item)}\s*\(", source)
        if function:
            arguments_end = _matching(source, function.end() - 1, "(", ")")
            body_start = source.find("{", arguments_end)
            semicolon = source.find(";", arguments_end)
            end = semicolon if 0 <= semicolon < body_start else body_start
            if end < 0:
                raise RuntimeError(f"cannot terminate external REST producer crate::{referenced_path}")
            reachable.update(
                name for name in re.findall(r"\b[A-Z][A-Za-z0-9_]*\b", source[function.start() : end])
                if name in wire_index
            )
    declarations: set[str] = set()
    queue = list(reachable)
    while queue:
        name = queue.pop()
        for declaration in wire_index.get(name, []):
            if declaration in declarations:
                continue
            declarations.add(declaration)
            queue.extend(
                nested for nested in re.findall(r"\b[A-Z][A-Za-z0-9_]*\b", declaration)
                if nested in wire_index
            )
    return sorted(declarations)


def discover_rest_schema_fingerprints(root: Path) -> dict[str, str]:
    catalog: list[str] = []
    handlers: dict[str, str] = {}
    external: dict[str, list[str]] = {}
    sources = _production_sources(root)
    wire_index = _wire_index(sources)
    for path in sorted((root / "src/api").rglob("*.rs")):
        if "tests" in path.parts or path.name == "tests.rs":
            continue
        text = mask_cfg_test_blocks(path.read_text(encoding="utf-8"))
        relative = path.relative_to(root).as_posix()
        for item in _wire_declarations(text):
            catalog.append(f"{relative}:{item}")
            catalog.extend(_serde_helper_evidence(item, sources))
        for name, signature in _handler_signatures(text).items():
            if name in handlers:
                raise RuntimeError(f"duplicate REST handler {name!r}")
            handlers[name] = f"{relative}:{signature}"
            external[name] = _external_wire_declarations(root, text, wire_index)
    if not handlers or not catalog:
        raise RuntimeError("REST wire-schema discovery found no handlers or serde/json declarations")
    shared = "\n".join(sorted(catalog))
    return {
        name: hashlib.sha256(
            f"{shared}\n{chr(10).join(sorted(external[name]))}\n{signature}".encode()
        ).hexdigest()
        for name, signature in handlers.items()
    }


def discover_rest_middleware_fingerprint(
    root: Path, router_sources: list[tuple[str, str]] | None = None,
) -> str | None:
    evidence: list[str] = []
    helpers: set[str] = set()
    sources = _production_sources(root)
    routed = router_sources or [(relative, source) for relative, source in sources if relative.startswith("src/api/")]
    for relative, source in routed:
        for match in re.finditer(r"\.(?:route_layer|layer)\s*\(", source):
            closing = _matching(source, match.end() - 1, "(", ")")
            call = _normalize(source[match.start() : closing + 1])
            evidence.append(f"{relative}:{call}")
            helpers.update(re.findall(r"\bfrom_fn\s*\(\s*([A-Za-z_][A-Za-z0-9_]*)", call))
    for helper in sorted(helpers):
        found = [
            f"{relative}:{item}"
            for relative, source in sources
            for item in _function_evidence(source, helper)
        ]
        if not found:
            raise RuntimeError(f"cannot resolve REST middleware helper {helper!r}")
        evidence.extend(found)
    return hashlib.sha256("\n".join(sorted(evidence)).encode()).hexdigest() if evidence else None


def reject_unsupported_router_methods(body: str, path: Path) -> None:
    found = [method for method in UNSUPPORTED_ROUTER_METHODS if re.search(rf"\.{method}\s*\(", body)]
    if found:
        raise RuntimeError(
            f"reachable Axum router in {path} uses unsupported registration methods: {', '.join(found)}"
        )


def reject_unsupported_method_routes(route: str, path: Path) -> None:
    calls = set(re.findall(r"(?:^|\.)\s*([a-z][a-z0-9_]*)\s*\(", route.strip()))
    unsupported = sorted((calls & UNSUPPORTED_METHOD_ROUTES) | {
        call for call in calls if call.endswith("_service") and call.removesuffix("_service") in SUPPORTED_ROUTE_METHODS
    })
    if unsupported:
        raise RuntimeError(
            f"reachable Axum route in {path} uses unsupported HTTP method registration: {', '.join(unsupported)}"
        )


def rest_surface_self_test() -> int:
    with tempfile.TemporaryDirectory(prefix="remem-rest-surface-") as raw:
        root = Path(raw)
        api = root / "src/api"
        api.mkdir(parents=True)
        external = root / "src/external"
        external.mkdir(parents=True)
        external_source = (
            "use serde::Serialize; #[derive(Serialize)] pub struct External { wire: bool }\n"
            "pub fn load() -> External { todo!() }\n"
        )
        (external / "mod.rs").write_text(external_source, encoding="utf-8")
        source = (
            "use serde::{Deserialize, Serialize};\n"
            "use crate::external::{load as fetch};\n"
            "#[derive(Deserialize)] struct Request { #[serde(deserialize_with = \"read_wire\")] value: String }\n"
            "#[derive(Serialize)] struct Response { #[serde(serialize_with = \"wire\")] ok: bool, #[serde(with = \"wire_module\")] wrapped: bool }\n"
            "fn wire<S>(value: &bool, serializer: S) { serializer.serialize_str(\"helper-key\"); }\n"
            "fn read_wire<'de, D>(deserializer: D) { parse(\"read-helper-key\"); }\n"
            "mod wire_module { fn serialize<S>() { emit(\"module-key\"); } fn deserialize<'de, D>() {} }\n"
            "struct Manual; impl Serialize for Manual { fn serialize(self) { serializer.serialize_str(\"manual-key\"); } }\n"
            "#[cfg(test)] mod tests { #[derive(Serialize)] struct TestOnly { ignored: bool } }\n"
            "pub(in crate::api) async fn handle_save() -> impl IntoResponse { private_work(); fetch(); if bad() { return map_error(); } (StatusCode::CREATED, Json(json!({\"ok\": true}))) }\n"
            "fn map_error() { error_response(StatusCode::BAD_REQUEST, \"stable-code\"); }\n"
        )
        (api / "save.rs").write_text(source, encoding="utf-8")
        before = discover_rest_schema_fingerprints(root)
        (api / "save.rs").write_text(source.replace("private_work();", "other_private_work();"), encoding="utf-8")
        if discover_rest_schema_fingerprints(root) != before:
            raise RuntimeError("private REST implementation body changed its schema fingerprint")
        (api / "save.rs").write_text(source.replace("ignored: bool", "test_only: String"), encoding="utf-8")
        if discover_rest_schema_fingerprints(root) != before:
            raise RuntimeError("cfg(test) REST schema changed a production fingerprint")
        (api / "save.rs").write_text(source, encoding="utf-8")
        (external / "mod.rs").write_text(external_source.replace("wire: bool", "renamed: bool"), encoding="utf-8")
        if discover_rest_schema_fingerprints(root) == before:
            raise RuntimeError("reachable external response type change did not alter REST fingerprint")
        (external / "mod.rs").write_text(external_source, encoding="utf-8")
        (api / "save.rs").write_text(source.replace("StatusCode::CREATED", "StatusCode::OK"), encoding="utf-8")
        if discover_rest_schema_fingerprints(root) == before:
            raise RuntimeError("REST success status change did not alter its schema fingerprint")
        (api / "save.rs").write_text(source.replace("stable-code", "renamed-code"), encoding="utf-8")
        if discover_rest_schema_fingerprints(root) == before:
            raise RuntimeError("REST stable error code change did not alter its schema fingerprint")
        (api / "save.rs").write_text(source.replace("manual-key", "renamed-key"), encoding="utf-8")
        if discover_rest_schema_fingerprints(root) == before:
            raise RuntimeError("custom serde wire-key change did not change its schema fingerprint")
        (api / "save.rs").write_text(source.replace("helper-key", "renamed-helper-key"), encoding="utf-8")
        if discover_rest_schema_fingerprints(root) == before:
            raise RuntimeError("serde field helper change did not change its schema fingerprint")
        (api / "save.rs").write_text(source.replace("read-helper-key", "renamed-read-key"), encoding="utf-8")
        if discover_rest_schema_fingerprints(root) == before:
            raise RuntimeError("serde deserialize helper change did not change its schema fingerprint")
        (api / "save.rs").write_text(source.replace("module-key", "renamed-module-key"), encoding="utf-8")
        if discover_rest_schema_fingerprints(root) == before:
            raise RuntimeError("serde helper module change did not change its schema fingerprint")
        (api / "save.rs").write_text(source.replace("value: String", "renamed: String"), encoding="utf-8")
        if discover_rest_schema_fingerprints(root) == before:
            raise RuntimeError("REST request field change did not change its schema fingerprint")
        (api / "save.rs").write_text(source.replace('{"ok": true}', '{"url": "https://old.example", "ok": true}'), encoding="utf-8")
        url_before = discover_rest_schema_fingerprints(root)
        (api / "save.rs").write_text(source.replace('{"ok": true}', '{"url": "https://new.example", "ok": true}'), encoding="utf-8")
        if discover_rest_schema_fingerprints(root) == url_before:
            raise RuntimeError("REST URL string change did not change its schema fingerprint")
        (api / "server.rs").write_text("fn require_api_token() { old_auth(); } fn router() { Router::new().route_layer(middleware::from_fn(require_api_token)); }", encoding="utf-8")
        middleware_before = discover_rest_middleware_fingerprint(root)
        (api / "server.rs").write_text("fn require_api_token() { new_auth(); } fn router() { Router::new().route_layer(middleware::from_fn(require_api_token)); }", encoding="utf-8")
        if discover_rest_middleware_fingerprint(root) == middleware_before:
            raise RuntimeError("REST authentication middleware change did not alter its fingerprint")
        rejected = False
        try:
            reject_unsupported_router_methods("Router::new().route_service(\"/x\", service)", api / "server.rs")
        except RuntimeError:
            rejected = True
        if not rejected:
            raise RuntimeError("unsupported Axum route_service registration did not fail closed")
        try:
            reject_unsupported_method_routes("get(handle).connect(tunnel)", api / "server.rs")
        except RuntimeError:
            pass
        else:
            raise RuntimeError("unsupported CONNECT method registration did not fail closed")
    print("REST surface fingerprint self-test: ok")
    return 0


if __name__ == "__main__":
    raise SystemExit(rest_surface_self_test())
