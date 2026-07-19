#!/usr/bin/env python3
"""Read-only telemetry collector for SpecRail runtime budgets (GH-137).

Scans a Codex session jsonl inside one tranche window and counts observable
runtime events. The output feeds `observed_compaction_count` and the other
observed counters in a checkpoint_version 3 budget before the runtime ledger
gate compares them against the declared limits.

Guarantees (product.md B-003 / B-009 / B-010):
- purely read-only: the session file is never written, no network calls;
- a missing or unreadable file returns `telemetry_source: unavailable` with
  no count fields — it never returns a fabricated 0 count;
- corrupt lines are skipped, but a window whose lines are all unparseable is
  treated as unreadable;
- counting is scoped to the tranche window: events before
  `--tranche-start-offset` (the session line count recorded when the tranche
  started) never leak into the new tranche's counters.
"""

from __future__ import annotations

import argparse
import json
import sys
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


COMPACTION_PAYLOAD_TYPES = {"context_compacted"}
COMPACTION_TOP_LEVEL_TYPES = {"compacted", "context_compacted"}
TOOL_CALL_EVENT_TYPES = {
    "custom_tool_call",
    "function_call",
    "local_shell_call",
    "mcp_tool_call",
    "tool_call",
    "web_search_call",
}


def _now_iso() -> str:
    return datetime.now(timezone.utc).isoformat()


def parse_timestamp(value: Any) -> datetime | None:
    if not isinstance(value, str) or not value.strip():
        return None
    raw = value.strip()
    if raw.endswith("Z"):
        raw = raw[:-1] + "+00:00"
    try:
        parsed = datetime.fromisoformat(raw)
    except ValueError:
        return None
    if parsed.tzinfo is None:
        parsed = parsed.replace(tzinfo=timezone.utc)
    return parsed


def _event_types(event: dict[str, Any]) -> tuple[Any, Any]:
    payload = event.get("payload")
    payload_type = payload.get("type") if isinstance(payload, dict) else None
    return payload_type, event.get("type")


def _is_compaction(event: dict[str, Any]) -> bool:
    payload_type, top_type = _event_types(event)
    return (
        payload_type in COMPACTION_PAYLOAD_TYPES
        or top_type in COMPACTION_TOP_LEVEL_TYPES
    )


def _is_tool_call(event: dict[str, Any]) -> bool:
    payload_type, top_type = _event_types(event)
    return payload_type in TOOL_CALL_EVENT_TYPES or top_type in TOOL_CALL_EVENT_TYPES


def _compaction_window_id(event: dict[str, Any], line_number: int) -> str:
    payload = event.get("payload")
    candidates = []
    if isinstance(payload, dict):
        candidates.extend([payload.get("window_id"), payload.get("id")])
    candidates.append(event.get("id"))
    for candidate in candidates:
        if isinstance(candidate, str) and candidate.strip():
            return candidate.strip()
    return f"line-{line_number}"


def _event_timestamp(event: dict[str, Any]) -> datetime | None:
    payload = event.get("payload")
    for candidate in [
        event.get("timestamp"),
        event.get("ts"),
        payload.get("timestamp") if isinstance(payload, dict) else None,
    ]:
        parsed = parse_timestamp(candidate)
        if parsed is not None:
            return parsed
    return None


def _unavailable(reason: str) -> dict[str, Any]:
    return {
        "telemetry_source": "unavailable",
        "reason": reason,
        "collected_at": _now_iso(),
    }


def collect(session_path: Path | str, tranche_start_offset: int = 0) -> dict[str, Any]:
    """Collect tranche-window telemetry from a session jsonl (read-only)."""

    if (
        isinstance(tranche_start_offset, bool)
        or not isinstance(tranche_start_offset, int)
        or tranche_start_offset < 0
    ):
        raise ValueError("tranche_start_offset must be a non-negative integer")

    path = Path(session_path)
    try:
        raw = path.read_text(encoding="utf-8", errors="replace")
    except OSError as exc:
        return _unavailable(f"cannot read session file: {exc}")

    lines = raw.splitlines()
    if tranche_start_offset > len(lines):
        return _unavailable(
            f"tranche_start_offset {tranche_start_offset} is beyond the "
            f"session length ({len(lines)} lines); a stale offset cannot "
            "produce trusted zero counts"
        )
    window_lines = lines[tranche_start_offset:]

    parsed_events: list[tuple[int, dict[str, Any]]] = []
    parse_failures = 0
    for line_number, line in enumerate(window_lines, start=tranche_start_offset + 1):
        stripped = line.strip()
        if not stripped:
            continue
        try:
            event = json.loads(stripped)
        except json.JSONDecodeError:
            parse_failures += 1
            continue
        if not isinstance(event, dict):
            parse_failures += 1
            continue
        parsed_events.append((line_number, event))

    if parse_failures > 0 and not parsed_events:
        return _unavailable(
            "session file has no parseable events in the tranche window"
        )

    observed_compaction_count = 0
    observed_tool_calls = 0
    last_compaction_window_id: str | None = None
    first_timestamp: datetime | None = None
    last_timestamp: datetime | None = None

    for line_number, event in parsed_events:
        if _is_compaction(event):
            observed_compaction_count += 1
            last_compaction_window_id = _compaction_window_id(event, line_number)
        if _is_tool_call(event):
            observed_tool_calls += 1
        timestamp = _event_timestamp(event)
        if timestamp is not None:
            if first_timestamp is None:
                first_timestamp = timestamp
            last_timestamp = timestamp

    observed_wall_clock_minutes = 0
    if first_timestamp is not None and last_timestamp is not None:
        delta_seconds = (last_timestamp - first_timestamp).total_seconds()
        if delta_seconds > 0:
            observed_wall_clock_minutes = int((delta_seconds + 59) // 60)

    return {
        "observed_compaction_count": observed_compaction_count,
        "observed_tool_calls": observed_tool_calls,
        "observed_wall_clock_minutes": observed_wall_clock_minutes,
        "telemetry_source": "session_log",
        "last_compaction_window_id": last_compaction_window_id,
        "tranche_window": {
            "start_line": tranche_start_offset,
            "end_line": len(lines),
        },
        "skipped_lines": parse_failures,
        "collected_at": _now_iso(),
    }


def main() -> int:
    parser = argparse.ArgumentParser(
        description=(
            "Collect tranche-window telemetry from a Codex session jsonl "
            "(read-only)."
        )
    )
    parser.add_argument("session_path", help="Path to the session jsonl file")
    parser.add_argument(
        "--tranche-start-offset",
        type=int,
        default=0,
        help=(
            "0-based session line count recorded when the current tranche "
            "started; events before this offset are not counted"
        ),
    )
    args = parser.parse_args()

    try:
        result = collect(args.session_path, args.tranche_start_offset)
    except ValueError as exc:
        print(json.dumps({"error": str(exc)}, indent=2, sort_keys=True))
        return 1

    print(json.dumps(result, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    sys.exit(main())
