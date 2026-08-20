#!/usr/bin/env python3
"""Keep new SQLite migrations to one concern.

A migration may evolve schema, or it may rewrite historical business rows.
It must not do both to unrelated tables. Trigger bodies are ignored because
they are schema, not upgrade-time DML.

Historical mixed files are allowlisted and must not gain new rewrite targets.
See docs/maintenance/migration-discipline.md.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
MIGRATIONS = ROOT / "src" / "migrations"

# Shipped files that already rewrite a table they do not create or alter.
# Values are the extra upgrade-time UPDATE/DELETE targets still permitted.
HISTORICAL_CROSS_TABLE_REWRITES = {
    "src/migrations/v068_session_rollup_followup_checkpoint.sql": frozenset(
        {"extraction_tasks"}
    ),
    "src/migrations/v083_retrieval_enrichment_budget.sql": frozenset(
        {"ai_usage_events"}
    ),
}

BOOKKEEPING_TABLES = {"sqlite_sequence"}
SQL_KEYWORDS = {
    "ABORT",
    "FAIL",
    "IGNORE",
    "INTO",
    "OF",
    "ON",
    "OR",
    "REPLACE",
    "ROLLBACK",
    "SET",
}

CREATE_TABLE_RE = re.compile(
    r"\bCREATE\s+(?:VIRTUAL\s+)?TABLE(?:\s+IF\s+NOT\s+EXISTS)?\s+[\"`]?(\w+)[\"`]?",
    re.I,
)
ALTER_TABLE_RE = re.compile(r"\bALTER\s+TABLE\s+[\"`]?(\w+)[\"`]?", re.I)
DROP_TABLE_RE = re.compile(
    r"\bDROP\s+TABLE(?:\s+IF\s+EXISTS)?\s+[\"`]?(\w+)[\"`]?",
    re.I,
)
RENAME_TO_RE = re.compile(r"\bRENAME\s+TO\s+[\"`]?(\w+)[\"`]?", re.I)
CREATE_INDEX_TABLE_RE = re.compile(
    r"\bCREATE\s+(?:UNIQUE\s+)?INDEX(?:\s+IF\s+NOT\s+EXISTS)?\s+"
    r"[\"`]?(?:\w+\.)?(?:\w+)[\"`]?\s+ON\s+[\"`]?(\w+)[\"`]?",
    re.I,
)
CREATE_TRIGGER_TABLE_RE = re.compile(
    r"\bCREATE\s+(?:TEMP(?:ORARY)?\s+)?TRIGGER"
    r"(?:\s+IF\s+NOT\s+EXISTS)?\s+[\"`]?(?:\w+\.)?(?:\w+)[\"`]?"
    r"(?:(?!\bBEGIN\b).)*?\bON\s+[\"`]?(\w+)[\"`]?.*?\bBEGIN\b",
    re.I | re.S,
)
UPDATE_RE = re.compile(r"\bUPDATE\s+(?:OR\s+\w+\s+)?[\"`]?(\w+)[\"`]?", re.I)
DELETE_RE = re.compile(r"\bDELETE\s+FROM\s+[\"`]?(\w+)[\"`]?", re.I)
SQL_TOKEN_RE = re.compile(
    r"--[^\n]*|/\*.*?\*/|'(?:''|[^'])*'|\"(?:\"\"|[^\"])*\"|"
    r"`(?:``|[^`])*`|\[[^\]]*\]|\b[A-Za-z_][A-Za-z0-9_]*\b|;",
    re.S,
)


def strip_line_comments(sql: str) -> str:
    lines = []
    for line in sql.splitlines():
        if line.lstrip().startswith("--"):
            continue
        lines.append(re.sub(r"--.*", "", line))
    return "\n".join(lines)


def strip_trigger_definitions(sql: str) -> str:
    tokens = list(SQL_TOKEN_RE.finditer(sql))
    ranges: list[tuple[int, int]] = []
    index = 0
    while index < len(tokens):
        if token_keyword(tokens[index]) != "CREATE":
            index += 1
            continue
        cursor = next_significant_token(tokens, index + 1)
        if cursor is not None and token_keyword(tokens[cursor]) in {"TEMP", "TEMPORARY"}:
            cursor = next_significant_token(tokens, cursor + 1)
        if cursor is None or token_keyword(tokens[cursor]) != "TRIGGER":
            index += 1
            continue

        begin = next_keyword(tokens, cursor + 1, "BEGIN")
        if begin is None:
            ranges.append((tokens[index].start(), len(sql)))
            break

        case_depth = 0
        end_pos: int | None = None
        cursor = begin + 1
        while cursor < len(tokens):
            keyword = token_keyword(tokens[cursor])
            if keyword == "CASE":
                case_depth += 1
            elif keyword == "END":
                if case_depth > 0:
                    case_depth -= 1
                else:
                    semicolon = next_significant_token(tokens, cursor + 1)
                    if semicolon is not None and tokens[semicolon].group(0) == ";":
                        end_pos = tokens[semicolon].end()
                    else:
                        end_pos = tokens[cursor].end()
                    break
            cursor += 1

        if end_pos is None:
            ranges.append((tokens[index].start(), len(sql)))
            break
        ranges.append((tokens[index].start(), end_pos))
        while index < len(tokens) and tokens[index].end() <= end_pos:
            index += 1

    pieces: list[str] = []
    pos = 0
    for start, end in ranges:
        pieces.append(sql[pos:start])
        pos = end
    pieces.append(sql[pos:])
    return "".join(pieces)


def token_keyword(token: re.Match[str]) -> str | None:
    value = token.group(0)
    if value and (value[0].isalpha() or value[0] == "_"):
        return value.upper()
    return None


def next_significant_token(tokens: list[re.Match[str]], start: int) -> int | None:
    for index in range(start, len(tokens)):
        value = tokens[index].group(0)
        if not value.startswith(("--", "/*")):
            return index
    return None


def next_keyword(
    tokens: list[re.Match[str]], start: int, expected: str
) -> int | None:
    cursor = start
    while (cursor := next_significant_token(tokens, cursor)) is not None:
        if token_keyword(tokens[cursor]) == expected:
            return cursor
        cursor += 1
    return None


def schema_tables(sql: str) -> set[str]:
    names: set[str] = set()
    for pattern in (
        CREATE_TABLE_RE,
        ALTER_TABLE_RE,
        DROP_TABLE_RE,
        RENAME_TO_RE,
        CREATE_INDEX_TABLE_RE,
        CREATE_TRIGGER_TABLE_RE,
    ):
        names.update(match.group(1) for match in pattern.finditer(sql))
    return names


def rewrite_tables(sql: str) -> set[str]:
    names: set[str] = set()
    for pattern in (UPDATE_RE, DELETE_RE):
        for match in pattern.finditer(sql):
            name = match.group(1)
            if name.upper() not in SQL_KEYWORDS:
                names.add(name)
    return names


def related_tables(schema: set[str]) -> set[str]:
    related = set(schema)
    related.update(BOOKKEEPING_TABLES)
    related.update(f"{name}_fts" for name in schema)
    related.update(
        name
        for name in schema
        if name.startswith("_") or name.endswith("_old") or name.endswith("_new")
    )
    return related


def extra_rewrite_tables(sql: str) -> set[str]:
    uncommented = strip_line_comments(sql)
    schema = schema_tables(uncommented)
    if not schema:
        return set()
    body = strip_trigger_definitions(uncommented)
    extras = rewrite_tables(body) - related_tables(schema)
    return {name for name in extras if not name.startswith("_")}


def check_migrations(root: Path = ROOT) -> list[str]:
    errors: list[str] = []
    seen: set[str] = set()
    migrations = root / "src" / "migrations"
    for path in sorted(migrations.glob("v*.sql")):
        rel = path.relative_to(root).as_posix()
        extras = extra_rewrite_tables(path.read_text(encoding="utf-8"))
        allowed = HISTORICAL_CROSS_TABLE_REWRITES.get(rel)
        if allowed is not None:
            seen.add(rel)
            unexpected = extras - allowed
            missing = allowed - extras
            if unexpected:
                errors.append(
                    f"{rel}: allowlisted migration gained unrelated rewrite(s) "
                    f"{sorted(unexpected)}; split them out"
                )
            if missing:
                errors.append(
                    f"{rel}: allowlisted rewrite target(s) {sorted(missing)} "
                    "disappeared; update HISTORICAL_CROSS_TABLE_REWRITES"
                )
            continue
        if extras:
            errors.append(
                f"{rel}: schema evolution plus unrelated upgrade-time rewrite "
                f"of {sorted(extras)}; keep one concern per migration "
                "(see docs/maintenance/migration-discipline.md)"
            )
    missing_files = sorted(set(HISTORICAL_CROSS_TABLE_REWRITES) - seen)
    if missing_files:
        errors.append(
            "allowlisted mixed migrations disappeared; update "
            "check_migration_concerns.py: " + ", ".join(missing_files)
        )
    return errors


def main() -> int:
    errors = check_migrations()
    if errors:
        print("migration concern check failed:", file=sys.stderr)
        for error in errors:
            print(f"  - {error}", file=sys.stderr)
        print("See docs/maintenance/migration-discipline.md.", file=sys.stderr)
        return 1
    print("migration concern check: ok")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
