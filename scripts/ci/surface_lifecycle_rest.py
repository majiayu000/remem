#!/usr/bin/env python3
"""Fingerprint REST wire declarations and reject untracked Axum registrations."""

from __future__ import annotations

import hashlib
import re
import tempfile
from pathlib import Path


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


def discover_rest_schema_fingerprints(root: Path) -> dict[str, str]:
    catalog: list[str] = []
    handlers: dict[str, str] = {}
    for path in sorted((root / "src/api").rglob("*.rs")):
        if "tests" in path.parts or path.name == "tests.rs":
            continue
        text = path.read_text(encoding="utf-8")
        relative = path.relative_to(root).as_posix()
        catalog.extend(f"{relative}:{item}" for item in _wire_declarations(text))
        for name, signature in _handler_signatures(text).items():
            if name in handlers:
                raise RuntimeError(f"duplicate REST handler {name!r}")
            handlers[name] = f"{relative}:{signature}"
    if not handlers or not catalog:
        raise RuntimeError("REST wire-schema discovery found no handlers or serde/json declarations")
    shared = "\n".join(sorted(catalog))
    return {
        name: hashlib.sha256(f"{shared}\n{signature}".encode()).hexdigest()
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
        source = (
            "use serde::{Deserialize, Serialize};\n"
            "#[derive(Deserialize)] struct Request { value: String }\n"
            "#[derive(Serialize)] struct Response { ok: bool }\n"
            "pub(in crate::api) async fn handle_save() -> impl IntoResponse { private_work(); Json(json!({\"ok\": true})) }\n"
        )
        (api / "save.rs").write_text(source, encoding="utf-8")
        before = discover_rest_schema_fingerprints(root)
        (api / "save.rs").write_text(source.replace("private_work();", "other_private_work();"), encoding="utf-8")
        if discover_rest_schema_fingerprints(root) != before:
            raise RuntimeError("private REST implementation body changed its schema fingerprint")
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
