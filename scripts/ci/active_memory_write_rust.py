"""Small Rust-literal scanners used by the active-memory write guard."""

from __future__ import annotations

import re

STRING_RE = re.compile(
    r'r(?P<hashes>#*)"(?P<raw>.*?)"(?P=hashes)|"(?P<normal>(?:\\.|[^"\\])*)"',
    re.S,
)
CHAR_RE = re.compile(r"'(?:\\.|[^'\\])'")


def decoded_literal(match: re.Match[str]) -> str:
    raw = match.group("raw")
    if raw is not None:
        return raw
    value = match.group("normal") or ""
    return re.sub(r"\\\s*\n\s*", "", value).replace(r'\"', '"')


def mask_rust_comments(text: str) -> str:
    """Erase Rust comments while preserving byte positions and string contents."""
    chars = list(text)
    index = 0
    while index < len(text):
        if text.startswith("//", index):
            end = text.find("\n", index)
            end = len(text) if end == -1 else end
            for cursor in range(index, end):
                chars[cursor] = " "
            index = end
            continue
        if text.startswith("/*", index):
            depth = 1
            cursor = index + 2
            while cursor < len(text) and depth:
                if text.startswith("/*", cursor):
                    depth += 1
                    cursor += 2
                elif text.startswith("*/", cursor):
                    depth -= 1
                    cursor += 2
                else:
                    cursor += 1
            for offset in range(index, cursor):
                if chars[offset] != "\n":
                    chars[offset] = " "
            index = cursor
            continue
        raw = re.match(r'r(?P<hashes>#*)"', text[index:])
        if raw:
            terminator = '"' + raw.group("hashes")
            end = text.find(terminator, index + raw.end())
            index = len(text) if end == -1 else end + len(terminator)
            continue
        if text[index] == '"':
            index += 1
            while index < len(text):
                if text[index] == "\\":
                    index += 2
                elif text[index] == '"':
                    index += 1
                    break
                else:
                    index += 1
            continue
        index += 1
    return "".join(chars)


def mask_rust_structure(text: str) -> str:
    """Erase comments and literals so structural braces remain unambiguous."""
    chars = list(mask_rust_comments(text))
    for match in STRING_RE.finditer(text):
        for offset in range(match.start(), match.end()):
            if chars[offset] != "\n":
                chars[offset] = " "
    structure = "".join(chars)
    for match in CHAR_RE.finditer(structure):
        for offset in range(match.start(), match.end()):
            chars[offset] = " "
    return "".join(chars)


def array_composition_spans(text: str) -> list[tuple[int, int]]:
    """Return array spans immediately consumed by `.concat()` or `.join()`."""
    spans: list[tuple[int, int]] = []
    for opening in (match.start() for match in re.finditer(r"\[", text)):
        depth = 1
        index = opening + 1
        while index < len(text) and depth:
            literal = STRING_RE.match(text, index)
            if literal:
                index = literal.end()
                continue
            if text[index] == "[":
                depth += 1
            elif text[index] == "]":
                depth -= 1
                if depth == 0:
                    tail = re.match(
                        r"\s*\)*\s*\.(?:concat\s*\(\s*\)|join\s*\()",
                        text[index + 1 :],
                    )
                    if tail:
                        spans.append((opening, index + 1))
                    break
            index += 1
    return spans


def add_expression_compositions(text: str) -> list[tuple[int, int, str]]:
    """Reconstruct literal fragments joined by Rust's binary `+` operator."""
    source = mask_rust_comments(text)

    def skip_space(index: int) -> int:
        while index < len(source) and source[index].isspace():
            index += 1
        return index

    def consume_owned_call(index: int) -> int:
        call = re.match(
            r"\s*\.\s*(?:to_owned|to_string)\s*\(\s*\)",
            source[index:],
        )
        return index if call is None else index + call.end()

    def parse_primary(index: int) -> tuple[list[str], int, int] | None:
        index = skip_space(index)
        literal = STRING_RE.match(source, index)
        if literal is not None:
            return [decoded_literal(literal)], consume_owned_call(literal.end()), 0

        string_from = re.match(r"String\s*::\s*from\s*\(", source[index:])
        if string_from is not None:
            inner = parse_addition(index + string_from.end())
            if inner is None:
                return None
            fragments, end, additions = inner
            end = skip_space(end)
            if end >= len(source) or source[end] != ")":
                return None
            return fragments, consume_owned_call(end + 1), additions

        if index < len(source) and source[index] == "(":
            inner = parse_addition(index + 1)
            if inner is None:
                return None
            fragments, end, additions = inner
            end = skip_space(end)
            if end >= len(source) or source[end] != ")":
                return None
            return fragments, consume_owned_call(end + 1), additions
        return None

    def parse_addition(index: int) -> tuple[list[str], int, int] | None:
        parsed = parse_primary(index)
        if parsed is None:
            return None
        fragments, end, additions = parsed
        while True:
            operator = skip_space(end)
            if (
                operator >= len(source)
                or source[operator] != "+"
                or source.startswith("+=", operator)
                or source.startswith("++", operator)
            ):
                break
            right = parse_primary(operator + 1)
            if right is None:
                break
            right_fragments, end, right_additions = right
            fragments.extend(right_fragments)
            additions += right_additions + 1
        return fragments, end, additions

    starts = {match.start() for match in STRING_RE.finditer(source)}
    starts.update(
        match.start()
        for match in re.finditer(r"\bString\s*::\s*from\s*\(", source)
    )
    starts.update(index for index, char in enumerate(source) if char == "(")
    candidates: list[tuple[int, int, str]] = []
    seen: set[tuple[int, int, str]] = set()
    for start in sorted(starts):
        parsed = parse_addition(start)
        if parsed is None:
            continue
        fragments, end, additions = parsed
        if additions == 0 or len(fragments) < 2:
            continue
        composition = (start, end, "".join(fragments))
        if composition not in seen:
            seen.add(composition)
            candidates.append(composition)
    return [
        (start, end, value)
        for start, end, value in candidates
        if not any(
            outer_start <= start
            and end <= outer_end
            and (outer_start, outer_end) != (start, end)
            for outer_start, outer_end, _ in candidates
        )
    ]


def append_compositions(text: str) -> list[tuple[int, int, str]]:
    """Reconstruct literal SQL assembled through local append operations."""
    structure = mask_rust_structure(text)
    let_re = re.compile(
        r"\blet\s+(?:mut\s+)?(?P<name>[A-Za-z_][A-Za-z0-9_]*)"
        r"(?:\s*:[^=;]+)?\s*=\s*?(?P<body>[^;]*);",
        re.S,
    )
    append_res = [
        re.compile(
            r"\b(?P<name>[A-Za-z_][A-Za-z0-9_]*)\s*\.\s*push_str\s*"
            r"\((?P<body>[^;]*)\)\s*;",
            re.S,
        ),
        re.compile(
            r"\b(?P<name>[A-Za-z_][A-Za-z0-9_]*)\s*\+=\s*?(?P<body>[^;]*);",
            re.S,
        ),
    ]
    operations = [
        (match.start(), "let", match) for match in let_re.finditer(structure)
    ]
    assignment_re = re.compile(
        r"\b(?P<name>[A-Za-z_][A-Za-z0-9_]*)\s*=(?!=)\s*?(?P<body>[^;]*);",
        re.S,
    )
    for match in assignment_re.finditer(structure):
        prefix = structure[max(0, match.start() - 16) : match.start()]
        if re.search(r"\blet\s+(?:mut\s+)?$", prefix):
            continue
        operations.append((match.start(), "assign", match))
    for append_re in append_res:
        operations.extend(
            (match.start(), "append", match)
            for match in append_re.finditer(structure)
        )

    builders: dict[tuple[tuple[int, ...], str], tuple[int, list[str]]] = {}
    compositions: list[tuple[int, int, str]] = []
    brace_stack: list[int] = []
    cursor = 0
    for offset, operation, match in sorted(operations, key=lambda item: item[0]):
        while cursor < offset:
            if structure[cursor] == "{":
                brace_stack.append(cursor)
            elif structure[cursor] == "}" and brace_stack:
                brace_stack.pop()
            cursor += 1
        scope = tuple(brace_stack)
        key = (scope, match.group("name"))
        body_end = match.end("body")
        conditional = re.search(
            r"\b(?:if|while)\s*$",
            structure[max(0, match.start() - 32) : match.start()],
        )
        if operation == "let" and conditional:
            opening = structure.find("{", match.start("body"), body_end)
            if opening != -1:
                scope += (opening,)
                key = (scope, match.group("name"))
                body_end = opening
        body = text[match.start("body") : body_end]
        fragments = [decoded_literal(literal) for literal in STRING_RE.finditer(body)]
        if operation == "let":
            builders[key] = (offset, fragments)
            continue
        candidates = [
            candidate
            for candidate in builders
            if candidate[1] == key[1] and scope[: len(candidate[0])] == candidate[0]
        ]
        if not candidates:
            continue
        key = max(candidates, key=lambda candidate: len(candidate[0]))
        if operation == "assign":
            assignment_body = structure[match.start("body") : body_end]
            self_addition = re.match(
                rf"\s*\(*\s*{re.escape(match.group('name'))}\s*\+",
                assignment_body,
            )
            if self_addition is None:
                if scope != key[0]:
                    continue
                builders[key] = (offset, fragments)
                continue
            start, accumulated = builders[key]
            accumulated.extend(fragments)
            compositions.append((start, match.end(), "".join(accumulated)))
            continue
        start, accumulated = builders[key]
        accumulated.extend(fragments)
        compositions.append((start, match.end(), "".join(accumulated)))
    return compositions
