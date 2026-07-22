#!/usr/bin/env python3
"""Extract one explicit issue relation from a PR snapshot."""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "checks"))

from github_evidence_common import EvidenceError  # noqa: E402
from github_issue_reference import (  # noqa: E402
    PARTIAL_ISSUE_REFERENCE_PATTERN,
    normalize_closing_issue_numbers,
    references_partial_issue,
)


def extract_issue(payload: Any, *, allow_closing: bool = False) -> int:
    if not isinstance(payload, dict) or set(payload) != {
        "body",
        "closingIssuesReferences",
    }:
        raise EvidenceError(
            "PR snapshot must contain exactly body and closingIssuesReferences"
        )
    closing = normalize_closing_issue_numbers(payload["closingIssuesReferences"])
    body = payload.get("body")
    if not isinstance(body, str) or not body.strip():
        raise EvidenceError("PR body must be a non-empty string")
    candidates = {
        int(match.group("number"))
        for match in PARTIAL_ISSUE_REFERENCE_PATTERN.finditer(body)
    }
    verified = sorted(
        issue for issue in candidates if references_partial_issue(body, issue)
    )
    if len(verified) > 1:
        raise EvidenceError(
            "PR body must contain exactly one visible standalone `Refs #<issue>` directive"
        )
    if verified:
        issue = verified[0]
        if issue in closing:
            raise EvidenceError(
                "implementation issue relation must be non-closing `Refs #<issue>`"
            )
        return issue
    if allow_closing and len(closing) == 1:
        return closing[0]
    if allow_closing:
        raise EvidenceError(
            "PR must contain exactly one standalone `Refs #<issue>` directive "
            "or exactly one closing issue relation"
        )
    raise EvidenceError(
        "PR body must contain exactly one visible standalone `Refs #<issue>` directive"
    )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("pr_json", type=Path)
    parser.add_argument(
        "--allow-closing",
        action="store_true",
        help="accept one closing issue when no standalone Refs directive exists",
    )
    args = parser.parse_args()
    try:
        payload = json.loads(args.pr_json.read_text(encoding="utf-8"))
        issue = extract_issue(payload, allow_closing=args.allow_closing)
    except (OSError, json.JSONDecodeError, EvidenceError) as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 1
    print(issue)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
