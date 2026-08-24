#!/usr/bin/env python3
"""Fingerprint REST wire declarations and reject untracked Axum registrations."""

from __future__ import annotations

import hashlib
import re
import tempfile
from pathlib import Path

from surface_lifecycle_evidence import mask_cfg_test_blocks


UNSUPPORTED_ROUTER_METHODS = (
    "route_service", "nest_service", "fallback", "fallback_service",
)


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
    value = re.sub(r"//[^\n]*|/\*.*?\*/", " ", value, flags=re.S)
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


def _handler_signatures(text: str) -> dict[str, str]:
    signatures: dict[str, str] = {}
    for match in re.finditer(
        r"pub\s*\(in\s+crate::api\)\s+async\s+fn\s+(?P<name>handle_[A-Za-z0-9_]+)\s*\(", text,
    ):
        arguments_end = _matching(text, match.end() - 1, "(", ")")
        body_start = text.find("{", arguments_end)
        if body_start < 0:
            raise RuntimeError(f"REST handler {match.group('name')} has no body")
        signatures[match.group("name")] = _normalize(text[match.start() : body_start])
    return signatures


def _wire_index(root: Path) -> dict[str, list[str]]:
    index: dict[str, list[str]] = {}
    for path in sorted((root / "src").rglob("*.rs")):
        if (
            "tests" in path.parts
            or path.name in {"tests.rs", "test_support.rs"}
            or path.stem.endswith("_tests")
        ):
            continue
        source = mask_cfg_test_blocks(path.read_text(encoding="utf-8"))
        relative = path.relative_to(root).as_posix()
        for declaration in _wire_declarations(source):
            named = re.search(
                r"\b(?:struct|enum)\s+([A-Za-z_][A-Za-z0-9_]*)|"
                r"\bimpl(?:\s*<[^>{}]*>)?\s+(?:serde::)?(?:Serialize|Deserialize(?:\s*<[^>{}]*>)?)"
                r"\s+for\s+([A-Za-z_][A-Za-z0-9_]*)",
                declaration,
            )
            if named:
                name = named.group(1) or named.group(2)
                index.setdefault(name, []).append(f"{relative}:{declaration}")
    return index


def _external_wire_declarations(
    root: Path,
    text: str,
    wire_index: dict[str, list[str]],
) -> list[str]:
    reachable = {
        name for name in re.findall(r"\b[A-Z][A-Za-z0-9_]*\b", text)
        if name in wire_index
    }
    for path_match in re.finditer(r"\bcrate::((?:[A-Za-z_][A-Za-z0-9_]*::)+[A-Za-z_][A-Za-z0-9_]*)", text):
        parts = path_match.group(1).split("::")
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
                raise RuntimeError(f"cannot terminate external REST producer {path_match.group(0)!r}")
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
    wire_index = _wire_index(root)
    for path in sorted((root / "src/api").rglob("*.rs")):
        if "tests" in path.parts or path.name == "tests.rs":
            continue
        text = mask_cfg_test_blocks(path.read_text(encoding="utf-8"))
        relative = path.relative_to(root).as_posix()
        catalog.extend(f"{relative}:{item}" for item in _wire_declarations(text))
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


def reject_unsupported_router_methods(body: str, path: Path) -> None:
    found = [method for method in UNSUPPORTED_ROUTER_METHODS if re.search(rf"\.{method}\s*\(", body)]
    if found:
        raise RuntimeError(
            f"reachable Axum router in {path} uses unsupported registration methods: {', '.join(found)}"
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
            "#[derive(Deserialize)] struct Request { value: String }\n"
            "#[derive(Serialize)] struct Response { ok: bool }\n"
            "struct Manual; impl Serialize for Manual { fn serialize(self) { serializer.serialize_str(\"manual-key\"); } }\n"
            "#[cfg(test)] mod tests { #[derive(Serialize)] struct TestOnly { ignored: bool } }\n"
            "pub(in crate::api) async fn handle_save() -> impl IntoResponse { private_work(); crate::external::load(); Json(json!({\"ok\": true})) }\n"
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
        (api / "save.rs").write_text(source.replace("manual-key", "renamed-key"), encoding="utf-8")
        if discover_rest_schema_fingerprints(root) == before:
            raise RuntimeError("custom serde wire-key change did not change its schema fingerprint")
        (api / "save.rs").write_text(source.replace("value: String", "renamed: String"), encoding="utf-8")
        if discover_rest_schema_fingerprints(root) == before:
            raise RuntimeError("REST request field change did not change its schema fingerprint")
        rejected = False
        try:
            reject_unsupported_router_methods("Router::new().route_service(\"/x\", service)", api / "server.rs")
        except RuntimeError:
            rejected = True
        if not rejected:
            raise RuntimeError("unsupported Axum route_service registration did not fail closed")
    print("REST surface fingerprint self-test: ok")
    return 0


if __name__ == "__main__":
    raise SystemExit(rest_surface_self_test())
