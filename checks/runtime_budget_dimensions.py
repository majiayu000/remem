#!/usr/bin/env python3
"""Hard-dimension budget judging for checkpoint_version 3 (GH-137).

Split out of runtime_gate_rules.py to keep that file within the size guard:
per-dimension override validation, append-only history verification, the
full-test head binding, and the hard budget dimension loop live here.
"""

from __future__ import annotations

from datetime import datetime
from typing import Any


OVERRIDE_DIMENSIONS = {
    "compaction",
    "wall_clock",
    "tool_calls",
    "review_rounds",
    "full_test_runs",
    "item_cap",
}
# (dimension, declared limit key, default limit, observed key)
HARD_BUDGET_DIMENSIONS = [
    ("wall_clock", "max_wall_clock_minutes", 120, "observed_wall_clock_minutes"),
    ("tool_calls", "max_tool_calls", 250, "observed_tool_calls"),
    (
        "review_rounds",
        "max_review_correction_rounds",
        2,
        "observed_review_correction_rounds",
    ),
    (
        "full_test_runs",
        "max_full_test_runs_per_head",
        1,
        "observed_full_test_runs_current_head",
    ),
]
# Clock skew tolerance before a future tranche_started_at is rejected.
FUTURE_TIMESTAMP_SKEW_SECONDS = 120


def _nonempty_string(value: Any) -> bool:
    return isinstance(value, str) and bool(value.strip())


def _nonneg_int(value: Any) -> int | None:
    if isinstance(value, bool) or not isinstance(value, int) or value < 0:
        return None
    return value


def judge_dimension(
    dimension: str,
    effective: int,
    limit: int,
    *,
    override_dimensions: set[str],
    source_display: str,
    errors: list[str],
    warnings: list[str],
    satisfied: list[str],
) -> None:
    """Judge one hard budget dimension against its limit.

    An overrun covered by a valid per-dimension override stays allowed but
    must remain visible: it lands in both satisfied and warnings so consumers
    can distinguish a clean pass from an audited overrun.
    """
    if effective <= limit:
        return
    if dimension in override_dimensions:
        satisfied.append(
            f"{dimension} budget exceeded under a recorded per-dimension "
            "budget_overrides entry"
        )
        warnings.append(
            f"budget overridden: {dimension} observed {effective} > "
            f"limit {limit} under a recorded per-dimension budget_overrides "
            f"entry (telemetry_source={source_display})"
        )
    else:
        errors.append(
            f"budget exceeded: {dimension} observed {effective} > "
            f"limit {limit} (telemetry_source={source_display})"
        )


def _validate_budget_overrides(budget: dict[str, Any], errors: list[str]) -> set[str]:
    """Return the dimensions covered by a valid budget_overrides entry."""
    if budget.get("budget_override") is not None:
        errors.append(
            "budget.budget_override is a version-2 structure; checkpoint_version 3 "
            "requires per-dimension budget_overrides entries"
        )
    overrides = budget.get("budget_overrides")
    if overrides is None:
        return set()
    if not isinstance(overrides, list):
        errors.append("budget.budget_overrides must be a list")
        return set()
    covered: set[str] = set()
    for index, entry in enumerate(overrides, start=1):
        if not isinstance(entry, dict):
            errors.append(f"budget.budget_overrides[{index}] must be an object")
            continue
        dimension = entry.get("dimension")
        if not isinstance(dimension, str) or dimension not in OVERRIDE_DIMENSIONS:
            allowed = ", ".join(sorted(OVERRIDE_DIMENSIONS))
            errors.append(
                f"budget.budget_overrides[{index}].dimension must be one of: {allowed}"
            )
            continue
        valid = True
        for key in ["scope", "conversation_marker"]:
            if not _nonempty_string(entry.get(key)):
                errors.append(
                    f"budget.budget_overrides[{index}].{key} must be a non-empty string"
                )
                valid = False
        if valid:
            covered.add(dimension)
    return covered


def _history_count(
    budget: dict[str, Any], key: str, errors: list[str], entry_matches: Any
) -> int | None:
    """Count matching append-only history entries; None means no history."""
    history = budget.get(key)
    if history is None:
        return None
    if not isinstance(history, list):
        errors.append(f"budget.{key} must be a list")
        return None
    count = 0
    for index, entry in enumerate(history, start=1):
        if not isinstance(entry, dict):
            errors.append(f"budget.{key}[{index}] must be an object")
        elif entry_matches(entry):
            count += 1
    return count


def _validate_full_test_head_binding(
    data: dict[str, Any],
    budget: dict[str, Any],
    observed: int | None,
    errors: list[str],
) -> str:
    """Enforce the head binding of the full-test counter (B-005)."""
    items = data.get("items")
    heads = {
        item["head_sha"].strip()
        for item in (items if isinstance(items, list) else [])
        if isinstance(item, dict) and _nonempty_string(item.get("head_sha"))
    }
    head_sha = budget.get("full_test_head_sha")
    head = head_sha.strip() if isinstance(head_sha, str) else ""
    if head_sha is not None and not head:
        errors.append("budget.full_test_head_sha must be a non-empty string")
        return ""
    counted = observed if isinstance(observed, int) else 0
    if not head:
        if counted > 0 or heads:
            errors.append(
                "budget.full_test_head_sha is required when "
                "observed_full_test_runs_current_head is above 0 or the tranche "
                "has a PR head under test"
            )
        return ""
    if not heads:
        if counted > 0:
            errors.append(
                "budget.full_test_head_sha is declared but the tranche has no PR "
                "head; omit the field and keep the full-test count at 0 instead "
                "of fabricating a SHA"
            )
            return ""
        return head
    if head not in heads:
        errors.append(
            f"budget.full_test_head_sha {head!r} does not match the current PR "
            "head; reset the count to 0 for the new head and update "
            "full_test_head_sha, keeping the old head's count in the append-only "
            "history"
        )
        return ""
    return head


def judge_hard_dimensions(
    data: dict[str, Any],
    budget: dict[str, Any],
    errors: list[str],
    warnings: list[str],
    satisfied: list[str],
    *,
    gate_now: datetime,
    tranche_started: datetime | None,
    observed_values: dict[str, int | None],
    override_dimensions: set[str],
    telemetry_trusted: bool,
    source_display: str,
) -> None:
    """Judge the four hard budget dimensions with provenance checks."""
    tranche_id = data.get("tranche_id")
    review_history = _history_count(
        budget,
        "review_correction_history",
        errors,
        lambda entry: entry.get("tranche_id") is None
        or entry.get("tranche_id") == tranche_id,
    )
    current_head = _validate_full_test_head_binding(
        data,
        budget,
        observed_values.get("observed_full_test_runs_current_head"),
        errors,
    )
    full_test_history = _history_count(
        budget,
        "full_test_history",
        errors,
        lambda entry: bool(current_head) and entry.get("head_sha") == current_head,
    )

    future_start = False
    if tranche_started is not None:
        skew_seconds = (tranche_started - gate_now).total_seconds()
        if skew_seconds > FUTURE_TIMESTAMP_SKEW_SECONDS:
            future_start = True
            errors.append(
                "checkpoint.tranche_started_at "
                f"{data.get('tranche_started_at')!r} is "
                f"{int(skew_seconds)}s in the future relative to the gate "
                "clock; a future tranche start cannot anchor the wall-clock "
                "budget and is rejected instead of clamped to zero elapsed"
            )

    for dimension, limit_key, default, observed_key in HARD_BUDGET_DIMENSIONS:
        limit_value = budget.get(limit_key)
        if limit_value is None:
            limit = default
        elif _nonneg_int(limit_value) is None or limit_value < 1:
            errors.append(f"budget.{limit_key} must be a positive integer")
            continue
        else:
            limit = limit_value
        observed = observed_values.get(observed_key)
        if observed is None:
            continue
        effective = observed
        if dimension == "wall_clock" and tranche_started is not None:
            if future_start:
                continue
            recomputed = int(
                max(0.0, (gate_now - tranche_started).total_seconds()) // 60
            )
            effective = max(observed, recomputed)
        elif dimension == "tool_calls" and not telemetry_trusted:
            warnings.append(
                "provenance: self_reported: tool_calls has no independent "
                f"telemetry source (telemetry_source={source_display})"
            )
        elif dimension == "review_rounds":
            if review_history is None:
                warnings.append(
                    "provenance: self_reported: review_rounds has no append-only "
                    "review_correction_history to verify against"
                )
            else:
                effective = max(observed, review_history)
        elif dimension == "full_test_runs":
            if full_test_history is None:
                warnings.append(
                    "provenance: self_reported: full_test_runs has no append-only "
                    "full_test_history to verify against"
                )
            else:
                effective = max(observed, full_test_history)
        judge_dimension(
            dimension,
            effective,
            limit,
            override_dimensions=override_dimensions,
            source_display=source_display,
            errors=errors,
            warnings=warnings,
            satisfied=satisfied,
        )
