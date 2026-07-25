#!/usr/bin/env python3
"""Shared structured rejection-item helpers for SpecRail gates (GH-141).

Every rejecting gate emits a machine-readable ``rejection_items`` array so a
caller can fix all defects in one round. Items are deterministic, deduplicated,
and comparable across rounds via ``--prior-rejection`` payloads.
"""

from __future__ import annotations

import argparse
import json
import re
from pathlib import Path
from typing import Any, Iterable


CATEGORIES = frozenset(
    {
        "missing_artifact",
        "invalid_state",
        "missing_evidence_field",
        "invalid_evidence_value",
        "contract_violation",
        "config_error",
    }
)

_PLACEHOLDER_VALUES = frozenset(
    {"", "n/a", "na", "unknown", "none", "null", "-", "tbd", "todo"}
)

_WHITESPACE_RE = re.compile(r"\s+")

_ITEM_ID_SUFFIX_RE = re.compile(r"#\d+$")


class RejectionItemError(ValueError):
    """Raised when a gate tries to build an invalid rejection item."""


def _slug(text: str) -> str:
    return _WHITESPACE_RE.sub("-", text.strip())


def make_item(category: str, subject: str, expected: str, found: str) -> dict[str, str]:
    """Build one validated rejection item with a stable ``item_id``."""

    if category not in CATEGORIES:
        allowed = ", ".join(sorted(CATEGORIES))
        raise RejectionItemError(
            f"rejection item category must be one of: {allowed}; got {category!r}"
        )
    for name, value in [("subject", subject), ("expected", expected), ("found", found)]:
        if not isinstance(value, str) or not value.strip():
            raise RejectionItemError(f"rejection item {name} must be a non-empty string")
    for name, value in [("expected", expected), ("found", found)]:
        if value.strip().lower() in _PLACEHOLDER_VALUES:
            raise RejectionItemError(
                f"rejection item {name} must be a concrete value description; "
                f"got placeholder {value!r}"
            )
    return {
        "item_id": f"{category}:{_slug(subject)}",
        "category": category,
        "expected": expected.strip(),
        "found": found.strip(),
    }


def item_from_missing(entry: str, category: str = "missing_evidence_field") -> dict[str, str]:
    """Convert one legacy ``missing`` entry into a rejection item."""

    return make_item(category, entry, f"{entry} present", "absent")


def _concrete_found(observed: str) -> str:
    """Normalize placeholder observations into a concrete ``found`` description.

    Legacy validators report values like ``...; got None`` or ``...; got ''``;
    passing those raw into :func:`make_item` trips the placeholder guard and
    degrades the whole result to one ``config_error``.
    """

    text = observed.strip()
    if text.lower() in _PLACEHOLDER_VALUES:
        return f"placeholder value {text!r} reported"
    return text


def item_from_reason(reason: str, category: str = "contract_violation") -> dict[str, str]:
    """Convert one legacy ``reasons`` entry into a rejection item."""

    requirement, sep, observed = reason.partition("; got ")
    if sep and requirement.strip() and observed.strip():
        return make_item(category, requirement, requirement, _concrete_found(observed))
    return make_item(
        category,
        reason,
        f"requirement satisfied: {reason}",
        f"requirement violated: {reason}",
    )


def items_from_legacy(
    missing: Iterable[str] = (),
    reasons: Iterable[str] = (),
    *,
    missing_category: str = "missing_evidence_field",
    reason_category: str = "contract_violation",
) -> list[dict[str, str]]:
    """Convert legacy missing/reasons string lists into rejection items."""

    items = [item_from_missing(entry, missing_category) for entry in missing]
    items.extend(item_from_reason(reason, reason_category) for reason in reasons)
    return items


def _validate_item_shape(item: Any) -> dict[str, str]:
    if not isinstance(item, dict):
        raise RejectionItemError("rejection item must be an object")
    for key in ["item_id", "category", "expected", "found"]:
        value = item.get(key)
        if not isinstance(value, str) or not value.strip():
            raise RejectionItemError(f"rejection item requires non-empty {key}")
    if item["category"] not in CATEGORIES:
        allowed = ", ".join(sorted(CATEGORIES))
        raise RejectionItemError(
            f"rejection item category must be one of: {allowed}; got {item['category']!r}"
        )
    return {key: item[key] for key in ["item_id", "category", "expected", "found"]}


def finalize_items(items: Iterable[dict[str, str]]) -> list[dict[str, str]]:
    """Deduplicate and deterministically order rejection items.

    Items sharing an ``item_id`` with identical ``(expected, found)`` merge into
    one entry. Differing pairs are all kept: the group is sorted by
    ``(expected, found)`` and each entry gets a ``#1``, ``#2``... suffix, so the
    output is independent of input order and loses no comparison data.
    """

    groups: dict[str, dict[str, Any]] = {}
    for raw in items:
        item = _validate_item_shape(raw)
        group = groups.setdefault(
            item["item_id"], {"category": item["category"], "pairs": set()}
        )
        group["pairs"].add((item["expected"], item["found"]))

    output: list[dict[str, str]] = []
    for item_id in groups:
        category = groups[item_id]["category"]
        pairs = sorted(groups[item_id]["pairs"])
        if len(pairs) == 1:
            expected, found = pairs[0]
            output.append(
                {
                    "item_id": item_id,
                    "category": category,
                    "expected": expected,
                    "found": found,
                }
            )
            continue
        for index, (expected, found) in enumerate(pairs, start=1):
            output.append(
                {
                    "item_id": f"{item_id}#{index}",
                    "category": category,
                    "expected": expected,
                    "found": found,
                }
            )
    return sorted(output, key=lambda entry: entry["item_id"])


def load_prior_rejection(
    path: str | Path,
) -> tuple[list[dict[str, str]] | None, dict[str, str] | None]:
    """Load a prior rejection payload; fail closed into a config_error item."""

    expected = "readable prior rejection payload containing rejection_items[]"
    try:
        data = json.loads(Path(path).read_text(encoding="utf-8"))
    except OSError as exc:
        return None, make_item(
            "config_error",
            "prior_rejection",
            expected,
            f"cannot read prior rejection file {path}: {exc}",
        )
    except json.JSONDecodeError as exc:
        return None, make_item(
            "config_error",
            "prior_rejection",
            expected,
            f"invalid prior rejection JSON {path}: {exc.msg}",
        )
    if not isinstance(data, dict) or not isinstance(data.get("rejection_items"), list):
        return None, make_item(
            "config_error",
            "prior_rejection",
            expected,
            f"prior rejection payload {path} lacks a rejection_items list",
        )
    items: list[dict[str, str]] = []
    for index, entry in enumerate(data["rejection_items"]):
        if not isinstance(entry, dict):
            return None, make_item(
                "config_error",
                "prior_rejection",
                expected,
                f"prior rejection payload {path} has a non-object entry at "
                f"rejection_items[{index}]",
            )
        items.append(
            {
                "item_id": str(entry.get("item_id") or ""),
                "category": str(entry.get("category") or ""),
                "expected": str(entry.get("expected") or ""),
                "found": str(entry.get("found") or ""),
            }
        )
    return items, None


def _base_item_id(item_id: str | None) -> str:
    """Strip a ``#N`` conflict suffix so ids compare stably across rounds."""

    return _ITEM_ID_SUFFIX_RE.sub("", item_id or "")


def repeat_rejection(
    current: Iterable[dict[str, str]], prior: Iterable[dict[str, str]]
) -> list[str]:
    """Return item_ids rejected identically (id+expected+found) in both rounds.

    Conflict suffixes (``#1``/``#2``) are stripped before comparison: an item
    suffixed in round one because of a same-id conflict still counts as a
    repeat when it survives unsuffixed after the conflicting sibling is fixed.
    """

    prior_triples = {
        (_base_item_id(item.get("item_id")), item.get("expected"), item.get("found"))
        for item in prior
    }
    return sorted(
        {
            item["item_id"]
            for item in current
            if (
                _base_item_id(item.get("item_id")),
                item.get("expected"),
                item.get("found"),
            )
            in prior_triples
        }
    )


def add_prior_rejection_argument(parser: argparse.ArgumentParser) -> None:
    parser.add_argument(
        "--prior-rejection",
        help="Prior round rejection payload JSON used to detect repeat rejections",
    )


def apply_prior_rejection(
    result: dict[str, Any],
    prior_path: str | None,
    *,
    blocked_actions: Iterable[str] = (),
) -> dict[str, Any]:
    """Compare this round's rejection_items against a prior payload.

    An unusable prior payload becomes a blocking ``config_error`` item (B-006)
    and blocks the caller-declared ``blocked_actions`` so the JSON stays
    self-consistent. Identical repeated items surface as a
    ``repeat_rejection`` section (B-005).
    """

    if not prior_path:
        return result
    prior_items, error_item = load_prior_rejection(prior_path)
    if error_item is not None:
        items = list(result.get("rejection_items") or [])
        items.append(error_item)
        result["rejection_items"] = finalize_items(items)
        result["decision"] = "blocked"
        merged_blocked = set(result.get("blocked_actions") or [])
        merged_blocked.update(blocked_actions)
        result["blocked_actions"] = sorted(merged_blocked)
        reasons = list(result.get("reasons") or [])
        reasons.append(f"--prior-rejection payload is unusable: {error_item['found']}")
        result["reasons"] = reasons
        return result
    repeats = repeat_rejection(result.get("rejection_items") or [], prior_items or [])
    if repeats:
        result["repeat_rejection"] = {"item_ids": repeats}
    return result
