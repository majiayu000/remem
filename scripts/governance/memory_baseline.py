#!/usr/bin/env python3
"""Emit a content-free, deterministic remem governance baseline.

The script deliberately allowlists aggregate fields from `remem status --json`.
It never copies project paths, memory text, raw-message content, failure paths,
or failure/drop details into the report.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import subprocess
import sys
from pathlib import Path
from typing import Any


REPORT_SCHEMA_VERSION = 1

TOTAL_FIELDS = ("memories", "observations", "sessions", "raw_messages")
REVIEW_FIELDS = (
    "pending",
    "median_age_secs",
    "max_age_secs",
    "inflow_7d",
    "resolved_7d",
)
RAW_ARCHIVE_FIELDS = ("messages", "ingest_failures", "parse_errors", "insert_errors")
CAPTURE_FIELDS = (
    "captured",
    "dropped",
    "unrecovered_spills",
    "extract_todo",
    "extract_running",
    "extract_expired",
    "extract_failed",
    "retryable_replay_ranges",
    "active_replay_ranges",
    "quarantined_replay_ranges",
    "pending_candidates",
    "pending_graph_candidates",
)
PROMOTION_FIELDS = (
    "source_kind",
    "review_status",
    "block_reason",
    "total",
    "last_7_days",
)


class BaselineError(ValueError):
    """Raised when status input does not satisfy the baseline contract."""


def require_object(value: Any, field: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise BaselineError(f"{field} must be a JSON object")
    return value


def require_array(value: Any, field: str) -> list[Any]:
    if not isinstance(value, list):
        raise BaselineError(f"{field} must be a JSON array")
    return value


def select_fields(source: dict[str, Any], fields: tuple[str, ...], section: str) -> dict[str, Any]:
    missing = [field for field in fields if field not in source]
    if missing:
        raise BaselineError(f"{section} missing required fields: {', '.join(missing)}")
    return {field: source[field] for field in fields}


def canonical_json(value: Any) -> bytes:
    return json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":")).encode(
        "utf-8"
    )


def build_report(status: dict[str, Any]) -> dict[str, Any]:
    totals = select_fields(
        require_object(status.get("totals"), "totals"), TOTAL_FIELDS, "totals"
    )
    review = select_fields(
        require_object(status.get("review_queue"), "review_queue"),
        REVIEW_FIELDS,
        "review_queue",
    )
    raw_archive = select_fields(
        require_object(status.get("raw_archive"), "raw_archive"),
        RAW_ARCHIVE_FIELDS,
        "raw_archive",
    )
    capture = select_fields(
        require_object(status.get("capture_pipeline"), "capture_pipeline"),
        CAPTURE_FIELDS,
        "capture_pipeline",
    )

    promotion = []
    for index, value in enumerate(
        require_array(status.get("candidate_promotion"), "candidate_promotion")
    ):
        promotion.append(
            select_fields(
                require_object(value, f"candidate_promotion[{index}]"),
                PROMOTION_FIELDS,
                f"candidate_promotion[{index}]",
            )
        )
    promotion.sort(
        key=lambda row: (
            str(row["source_kind"]),
            str(row["review_status"]),
            "" if row["block_reason"] is None else str(row["block_reason"]),
        )
    )

    metrics = {
        "totals": totals,
        "review_queue": review,
        "candidate_promotion": promotion,
        "raw_archive": raw_archive,
        "capture_pipeline": capture,
    }
    runtime_version = status.get("version")
    if not isinstance(runtime_version, str) or not runtime_version.strip():
        raise BaselineError("version must be a non-empty string")

    return {
        "schema_version": REPORT_SCHEMA_VERSION,
        "runtime_version": runtime_version,
        "metrics_sha256": hashlib.sha256(canonical_json(metrics)).hexdigest(),
        "metrics": metrics,
    }


def load_status(args: argparse.Namespace) -> dict[str, Any]:
    if args.status_json is not None:
        try:
            value = json.loads(args.status_json.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError) as error:
            raise BaselineError(f"cannot read status JSON: {error}") from error
    else:
        completed = subprocess.run(
            [args.remem, "status", "--json"],
            check=False,
            capture_output=True,
            text=True,
        )
        if completed.returncode != 0:
            detail = completed.stderr.strip() or f"exit status {completed.returncode}"
            raise BaselineError(f"remem status failed: {detail}")
        try:
            value = json.loads(completed.stdout)
        except json.JSONDecodeError as error:
            raise BaselineError(f"remem status returned invalid JSON: {error}") from error
    return require_object(value, "status")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="emit a content-free deterministic governance baseline"
    )
    source = parser.add_mutually_exclusive_group()
    source.add_argument(
        "--status-json",
        type=Path,
        help="read a previously captured remem status JSON instead of invoking remem",
    )
    source.add_argument(
        "--remem",
        default="remem",
        help="remem executable used for the read-only status call (default: remem)",
    )
    return parser.parse_args()


def main() -> int:
    try:
        report = build_report(load_status(parse_args()))
    except BaselineError as error:
        print(f"error: {error}", file=sys.stderr)
        return 2
    print(json.dumps(report, ensure_ascii=False, sort_keys=True, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
