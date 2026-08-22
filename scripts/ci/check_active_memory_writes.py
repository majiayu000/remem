#!/usr/bin/env python3
"""Reject curated-memory activation that bypasses the GH969 boundary."""

from __future__ import annotations

import argparse
import hashlib
import re
import sys
import tempfile
from collections import Counter, defaultdict
from dataclasses import dataclass
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]

# Raw SQL is permitted only in these reviewed boundary implementations. Every
# ordinary caller must construct ActiveMemoryWriteRequest and call execute_one.
ALLOWED_PRODUCTION_FILES = {
    "src/memory/store/write.rs": "permit-gated canonical row writer",
    "src/memory/store/write/activation.rs": "permit-gated immutable replacement writer",
    "src/memory_candidate/apply/write.rs": "candidate promotion writer called inside execute_one",
    "src/cli/actions/pack_import/active_import.rs": "governed pack safe-add closure",
    "src/memory/scope_cleanup/plan.rs": "validated cleanup-plan activation closure",
    "src/memory/governance.rs": "Web restore activation closure",
    "src/memory_candidate/review/approval.rs": "exact Dream stock recovery closure",
    "src/memory/lifecycle.rs": "Rust lifecycle replacement activation closure",
    "src/dream/apply.rs": "Dream consolidation calls the permit-gated canonical writer",
    "src/memory/lesson.rs": "lesson save calls the permit-gated canonical writer",
    "src/memory/procedure/mod.rs": "verified procedure promotion calls the permit-gated canonical writer",
    "src/cli/actions/import.rs": "governed best-effort backup import closure",
    "src/cli/actions/markdown_archive/persist.rs": "operator markdown activation closure",
    "src/memory/scope_cleanup/mutate.rs": "archive-only dynamic status helper cannot receive active",
}

ALLOWED_FIXTURE_FILES = {
    "src/eval/current_memory_contracts/fixture.rs": "offline evaluation fixture database",
    "src/eval/golden/run.rs": "offline golden-corpus fixture status downgrade",
    "src/eval/governance/fixture.rs": "offline governance fixture database",
    "src/eval/injection/run.rs": "offline injection fixture database",
    "src/eval/memory_bench/runner.rs": "offline benchmark fixture status downgrade",
    "src/worker/tests/rule_compilation.rs": "standalone worker test fixture",
}

# Statement/helper signatures are SHA-256 prefixes of normalized reviewed
# source literals. Counts are part of the baseline so copying an allowed write
# elsewhere in the same file still fails review.
EXPECTED_ALLOWED_FINDINGS = {
    "src/cli/actions/import.rs": {"memory_insert:8c332be8a69cc638": 1},
    "src/cli/actions/markdown_archive/persist.rs": {
        "active_status_update:cefa82c27f14e112": 1,
        "memory_insert:3b2c23d4790ac4d5": 1,
    },
    "src/cli/actions/pack_import/active_import.rs": {"memory_insert:b32f083c20611900": 1},
    "src/dream/apply.rs": {"raw_active_helper_call:61be54b164a77baa": 1},
    "src/eval/current_memory_contracts/fixture.rs": {
        "memory_insert:5aaf4428641cac9e": 1,
        "memory_insert:c96fd8deada30f23": 1,
    },
    "src/eval/golden/run.rs": {"active_status_update:17305831b37dc716": 1},
    "src/eval/governance/fixture.rs": {"active_status_update:1e35d404730bc56c": 1},
    "src/eval/injection/run.rs": {
        "memory_insert:34c775f6927e7a3e": 1,
        "memory_insert:e5bb2169da4f9fe2": 1,
        "memory_insert:eedce613d0319c52": 1,
        "memory_insert:fb8cd2c3717625eb": 1,
    },
    "src/eval/memory_bench/runner.rs": {"active_status_update:17305831b37dc716": 1},
    "src/memory/governance.rs": {
        "active_status_update:286bbbd2d3737bf8": 1,
        "active_status_update:d0ee564fdbd933ea": 1,
    },
    "src/memory/lesson.rs": {"raw_active_helper_call:61be54b164a77baa": 1},
    "src/memory/lifecycle.rs": {"memory_insert:cdd0c80c9f639249": 1},
    "src/memory/scope_cleanup/mutate.rs": {"active_status_update:8ff17e02fe62e4da": 1},
    "src/memory/scope_cleanup/plan.rs": {"active_status_update:4834b85f590b6a90": 1},
    "src/memory/store/write.rs": {
        "active_status_update:d60fd43cc23c0d09": 1,
        "memory_insert:91495e4782c3bb3f": 1,
        "raw_active_helper_call:61be54b164a77baa": 2,
    },
    "src/memory/store/write/activation.rs": {"memory_insert:901ea26ce30face5": 1},
    "src/memory_candidate/apply/write.rs": {"memory_insert:f55659946a93087e": 1},
    "src/memory_candidate/review/approval.rs": {"active_status_update:61bbe5295f20e99c": 1},
    "src/worker/tests/rule_compilation.rs": {"memory_insert:d3403c9d24c34a46": 1},
}

STRING_RE = re.compile(
    r'r(?P<hashes>#*)"(?P<raw>.*?)"(?P=hashes)|"(?P<normal>(?:\\.|[^"\\])*)"',
    re.S,
)
INSERT_RE = re.compile(r"\bINSERT\s+(?:OR\s+\w+\s+)?INTO\s+memories\b", re.I)
ACTIVE_UPDATE_RE = re.compile(
    r'''\bUPDATE\s+memories\b(?:(?!\bWHERE\b).)*?\bstatus\s*=\s*(?:[\"']active[\"']|\?\d*|:\w+)''',
    re.I | re.S,
)
RAW_HELPER_RE = re.compile(r"\binsert_memory_full_activated\s*\(")
RAW_HELPER_ALIAS_RE = re.compile(
    r"\binsert_memory_full_activated\s+as\s+([A-Za-z_][A-Za-z0-9_]*)"
)
RAW_HELPER_BIND_RE = re.compile(
    r"\blet\s+(?:mut\s+)?\(*\s*(?P<alias>[A-Za-z_][A-Za-z0-9_]*)\s*\)*"
    r"(?:\s*:[^=;]+)?\s*=\s*\(*\s*(?:[A-Za-z_][A-Za-z0-9_]*::)*"
    r"insert_memory_full_activated\b\s*\)*(?!\s*\()"
)
COMPOSED_SQL_RE = re.compile(r"\b(?:concat|format)\s*!\s*\((?P<body>[^;]*)\)", re.S)
CFG_TEST_MOD_RE = re.compile(r"#\s*\[\s*cfg\s*\(\s*test\s*\)\s*]\s*mod\s+\w+\s*\{")
MOD_RE = re.compile(
    r"^[ \t]*(?P<attrs>(?:#\s*\[[^]]+\]\s*)*)"
    r"(?:pub(?:\s*\([^)]*\))?\s+)?mod\s+(?P<name>[A-Za-z_][A-Za-z0-9_]*)\s*;"
    , re.M
)


@dataclass(frozen=True)
class Finding:
    path: str
    line: int
    kind: str
    signature: str


def finding_signature(kind: str, value: str) -> str:
    normalized = re.sub(r"\s+", " ", value).strip().lower()
    digest = hashlib.sha256(normalized.encode("utf-8")).hexdigest()[:16]
    return f"{kind}:{digest}"


def matching_brace(text: str, opening: int) -> int | None:
    depth = 0
    index = opening
    in_string = False
    escaped = False
    while index < len(text):
        char = text[index]
        if in_string:
            if escaped:
                escaped = False
            elif char == "\\":
                escaped = True
            elif char == '"':
                in_string = False
        elif char == '"':
            in_string = True
        elif char == "{":
            depth += 1
        elif char == "}":
            depth -= 1
            if depth == 0:
                return index
        index += 1
    return None


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


def erase_inline_test_modules(text: str) -> str:
    chars = list(text)
    structure = mask_rust_comments(text)
    for match in CFG_TEST_MOD_RE.finditer(structure):
        opening = structure.find("{", match.start(), match.end())
        closing = matching_brace(structure, opening)
        if closing is None:
            continue
        chars[match.start() : closing + 1] = " " * (closing + 1 - match.start())
    return "".join(chars)


def resolve_module(parent: Path, name: str) -> Path | None:
    base = (
        parent.parent
        if parent.name in {"lib.rs", "main.rs", "mod.rs"}
        else parent.with_suffix("")
    )
    direct = base / f"{name}.rs"
    nested = base / name / "mod.rs"
    if direct.exists():
        return direct
    if nested.exists():
        return nested
    return None


def proven_test_files(root: Path) -> set[Path]:
    discovered: set[tuple[Path, bool]] = set()
    test_only: set[Path] = set()
    queue: list[tuple[Path, bool]] = [(root / "src/lib.rs", False)]
    while queue:
        path, inherited_test = queue.pop()
        key = (path, inherited_test)
        if key in discovered or not path.exists():
            continue
        discovered.add(key)
        if inherited_test:
            test_only.add(path)
        text = mask_rust_comments(path.read_text(encoding="utf-8"))
        for match in MOD_RE.finditer(text):
            attrs = match.group("attrs")
            explicit_path = re.search(r'path\s*=\s*"([^"]+)"', attrs)
            child = (
                path.parent / explicit_path.group(1)
                if explicit_path
                else resolve_module(path, match.group("name"))
            )
            if child is None:
                continue
            child_test = inherited_test or bool(
                re.search(r"cfg\s*\([^]]*\btest\b", attrs)
            )
            queue.append((child, child_test))
    return test_only


def decoded_literal(match: re.Match[str]) -> str:
    raw = match.group("raw")
    if raw is not None:
        return raw
    value = match.group("normal") or ""
    return re.sub(r"\\\s*\n\s*", "", value).replace(r'\"', '"')


def normalize_sql(value: str) -> str:
    value = re.sub(r"/\*.*?\*/", " ", value, flags=re.S)
    value = re.sub(r"--[^\n]*", " ", value)
    return value


def array_concat_spans(text: str) -> list[tuple[int, int]]:
    """Return array-expression spans immediately consumed by `.concat()`.

    The small scanner skips Rust string literals, so punctuation such as `;`
    and `]` inside SQL fragments cannot hide an array concatenation.
    """
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
                    tail = re.match(r"\s*\)*\s*\.concat\s*\(\s*\)", text[index + 1 :])
                    if tail:
                        spans.append((opening, index + 1))
                    break
            index += 1
    return spans


def scan_rust(path: Path, rel: str) -> list[Finding]:
    text = erase_inline_test_modules(path.read_text(encoding="utf-8"))
    findings: list[Finding] = []
    for match in STRING_RE.finditer(text):
        literal = normalize_sql(decoded_literal(match))
        kind = None
        if INSERT_RE.search(literal):
            kind = "memory_insert"
        elif ACTIVE_UPDATE_RE.search(literal):
            kind = "active_status_update"
        if kind:
            findings.append(
                Finding(
                    rel,
                    text.count("\n", 0, match.start()) + 1,
                    kind,
                    finding_signature(kind, literal),
                )
            )
    for match in RAW_HELPER_RE.finditer(text):
        prefix = text[max(0, match.start() - 20) : match.start()]
        if "fn " in prefix:
            continue
        findings.append(
            Finding(
                rel,
                text.count("\n", 0, match.start()) + 1,
                "raw_active_helper_call",
                finding_signature("raw_active_helper_call", "insert_memory_full_activated"),
            )
        )
    for alias_match in RAW_HELPER_ALIAS_RE.finditer(text):
        alias = alias_match.group(1)
        for call in re.finditer(rf"\b{re.escape(alias)}\s*\(", text[alias_match.end() :]):
            start = alias_match.end() + call.start()
            findings.append(
                Finding(
                    rel,
                    text.count("\n", 0, start) + 1,
                    "raw_active_helper_call",
                    finding_signature("raw_active_helper_call", "insert_memory_full_activated"),
                )
            )
    for bind_match in RAW_HELPER_BIND_RE.finditer(text):
        findings.append(
            Finding(
                rel,
                text.count("\n", 0, bind_match.start()) + 1,
                "raw_active_helper_call",
                finding_signature("raw_active_helper_call", "insert_memory_full_activated"),
            )
        )
    for composed in COMPOSED_SQL_RE.finditer(text):
        fragments = [decoded_literal(match) for match in STRING_RE.finditer(composed.group("body"))]
        joined = normalize_sql("".join(fragments))
        kind = None
        if INSERT_RE.search(joined):
            kind = "memory_insert"
        elif ACTIVE_UPDATE_RE.search(joined):
            kind = "active_status_update"
        if kind and not any(
            finding.line == text.count("\n", 0, composed.start()) + 1 and finding.kind == kind
            for finding in findings
        ):
            findings.append(
                Finding(
                    rel,
                    text.count("\n", 0, composed.start()) + 1,
                    kind,
                    finding_signature(kind, joined),
                )
            )
    structure = mask_rust_comments(text)
    for start, end in array_concat_spans(structure):
        fragments = [decoded_literal(match) for match in STRING_RE.finditer(text[start:end])]
        joined = normalize_sql("".join(fragments))
        kind = None
        if INSERT_RE.search(joined):
            kind = "memory_insert"
        elif ACTIVE_UPDATE_RE.search(joined):
            kind = "active_status_update"
        if kind:
            findings.append(
                Finding(
                    rel,
                    text.count("\n", 0, start) + 1,
                    kind,
                    finding_signature(kind, joined),
                )
            )
    return findings


def scan_tree(root: Path) -> list[Finding]:
    test_files = proven_test_files(root)
    findings: list[Finding] = []
    for path in sorted((root / "src").rglob("*.rs")):
        if path in test_files:
            continue
        rel = path.relative_to(root).as_posix()
        findings.extend(scan_rust(path, rel))
    return findings


def check(root: Path = ROOT) -> list[str]:
    errors: list[str] = []
    allowed = {**ALLOWED_PRODUCTION_FILES, **ALLOWED_FIXTURE_FILES}
    findings = scan_tree(root)
    if root != ROOT:
        for finding in findings:
            errors.append(
                f"{finding.path}:{finding.line}: {finding.kind} bypasses ActiveMemoryWriteRequest; "
                "route the operation through memory::activation::execute_one"
            )
        return errors

    actual_allowed: dict[str, Counter[str]] = defaultdict(Counter)
    for finding in findings:
        if finding.path in allowed:
            actual_allowed[finding.path][finding.signature] += 1
            continue
        errors.append(
            f"{finding.path}:{finding.line}: {finding.kind} bypasses ActiveMemoryWriteRequest; "
            "route the operation through memory::activation::execute_one"
        )
    for path in sorted(allowed):
        actual = actual_allowed.get(path, Counter())
        expected = Counter(EXPECTED_ALLOWED_FINDINGS.get(path, {}))
        if actual != expected:
            errors.append(
                f"{path}: reviewed active-write signatures changed: "
                f"expected={dict(sorted(expected.items()))} actual={dict(sorted(actual.items()))}; "
                "inspect the exact sites and update the reviewed baseline"
            )
    return errors


def self_test() -> int:
    with tempfile.TemporaryDirectory(prefix="remem-active-write-guard-") as raw:
        root = Path(raw)
        (root / "src").mkdir()
        (root / "src/lib.rs").write_text("mod direct;\n#[cfg(test)] mod fixture;\n", encoding="utf-8")
        (root / "src/direct.rs").write_text(
            'fn bypass() {\n'
            ' let _ = "INSERT INTO memories (status) VALUES (\'active\')";\n'
            ' let _ = "UPDATE memories SET status = \'active\' WHERE id = ?1";\n'
            '}\n',
            encoding="utf-8",
        )
        (root / "src/dynamic.rs").write_text(
            'use crate::writer::insert_memory_full_activated as raw_write;\n'
            'fn bypass() {\n'
            ' let _ = concat!("INSERT INTO ", "memories (status) VALUES (\'active\')");\n'
            ' let _ = "UPDATE /* hidden */ memories SET status = \\"active\\" WHERE id = ?1";\n'
            ' raw_write();\n'
            ' let writer = crate::writer::insert_memory_full_activated; writer();\n'
            ' let grouped = (crate::writer::insert_memory_full_activated); grouped();\n'
            ' let (pattern) = (crate::writer::insert_memory_full_activated); pattern();\n'
            ' let _ = ["INSERT INTO ", "memories (status) VALUES (\'active\')"].concat();\n'
            ' let _ = [r#"INSERT INTO "#, r#"memories(status) VALUES (1);"#].concat();\n'
            ' let _ = (["INSERT INTO ", "memories(status) VALUES (]);"]).concat();\n'
            ' let _ = r##########"UPDATE memories SET status = "active" WHERE id = 1"##########;\n'
            '}\n',
            encoding="utf-8",
        )
        (root / "src/lib.rs").write_text(
            "mod direct;\nmod dynamic;\n// #[cfg(test)]\nmod commented;\n#[cfg(test)] mod fixture;\n",
            encoding="utf-8",
        )
        (root / "src/commented.rs").write_text(
            'fn bypass() { let _ = "INSERT INTO memories (status) VALUES (\'active\')"; }\n',
            encoding="utf-8",
        )
        (root / "src/fixture.rs").write_text(
            'fn seed() { let _ = "INSERT INTO memories (status) VALUES (\'active\')"; }\n',
            encoding="utf-8",
        )
        errors = check(root)
        direct_errors = [error for error in errors if "src/direct.rs" in error]
        dynamic_errors = [error for error in errors if "src/dynamic.rs" in error]
        commented_errors = [error for error in errors if "src/commented.rs" in error]
        if (
            len(errors) != 13
            or len(direct_errors) != 2
            or len(dynamic_errors) != 10
            or len(commented_errors) != 1
        ):
            print(f"active-memory guard self-test failed: {errors}", file=sys.stderr)
            return 1
    print("active-memory write guard self-test: ok")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        return self_test()
    errors = check()
    if errors:
        print("active-memory write guard failed:", file=sys.stderr)
        for error in errors:
            print(f"  - {error}", file=sys.stderr)
        return 1
    print("active-memory write guard: ok")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
