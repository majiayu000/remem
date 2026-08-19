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
    "src/migrations/v069_job_queue_atomicity.sql": frozenset({"jobs"}),
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
UPDATE_RE = re.compile(r"\bUPDATE\s+(?:OR\s+\w+\s+)?[\"`]?(\w+)[\"`]?", re.I)
DELETE_RE = re.compile(r"\bDELETE\s+FROM\s+[\"`]?(\w+)[\"`]?", re.I)
CREATE_TRIGGER_RE = re.compile(r"\bCREATE\s+TRIGGER\b", re.I)
BEGIN_RE = re.compile(r"\bBEGIN\b", re.I)
END_RE = re.compile(r"\bEND\s*;", re.I)


def strip_line_comments(sql: str) -> str:
    lines = []
    for line in sql.splitlines():
        if line.lstrip().startswith("--"):
            continue
        lines.append(re.sub(r"--.*", "", line))
    return "\n".join(lines)


def strip_trigger_definitions(sql: str) -> str:
    pieces: list[str] = []
    pos = 0
    for match in CREATE_TRIGGER_RE.finditer(sql):
        pieces.append(sql[pos : match.start()])
        begin = BEGIN_RE.search(sql, match.start())
        end = END_RE.search(sql, begin.start() if begin else match.start())
        if begin is None or end is None:
            pieces.append(sql[match.start() :])
            return "".join(pieces)
        pos = end.end()
    pieces.append(sql[pos:])
    return "".join(pieces)


def schema_tables(sql: str) -> set[str]:
    names: set[str] = set()
    for pattern in (CREATE_TABLE_RE, ALTER_TABLE_RE, DROP_TABLE_RE, RENAME_TO_RE):
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
    body = strip_trigger_definitions(strip_line_comments(sql))
    schema = schema_tables(body)
    if not schema:
        return set()
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
