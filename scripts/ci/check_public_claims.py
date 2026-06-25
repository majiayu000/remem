#!/usr/bin/env python3
"""Guard README/release surfaces against unsupported public benchmark claims."""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
BASELINE_REPORT = ROOT / "eval/public/reports/baseline.json"

CLAIM_SURFACES = [
    "README.md",
    "README.zh-CN.md",
    "CHANGELOG.md",
    "docs/release-lifecycle.md",
]

STRONG_CLAIM_RE = re.compile(
    r"\b("
    r"SOTA|state[- ]of[- ]the[- ]art|best|beats?|outperforms?|"
    r"superior(?:ity)?|coding[- ]task superiority|coding[- ]agent outcome improvement"
    r")\b",
    re.I,
)

CODING_CLAIM_RE = re.compile(
    r"\b("
    r"beats?|outperforms?|superior(?:ity)?|coding[- ]task superiority|"
    r"coding[- ]agent outcome improvement|maintained context file|MEMORY\.md"
    r")\b",
    re.I,
)

SOTA_CLAIM_RE = re.compile(r"\b(SOTA|state[- ]of[- ]the[- ]art|best)\b", re.I)

CONSERVATIVE_CONTEXT_RE = re.compile(
    r"\b("
    r"do not|don't|does not|must not|cannot|forbidden|unsupported|"
    r"directional|no public claim|not evaluated|not support|not claim|"
    r"until|unless|requires?|required|gate|guard|policy|stop-loss|"
    r"claim level|allowed claim|public claim policy|public SOTA claim|"
    r"honest claim|passes only|applies to|wording that says|"
    r"stop-loss signal|"
    r"evidence required|before that claim|not evidence"
    r")\b",
    re.I,
)

REPORT_LINK_RE = re.compile(
    r"(eval/public/reports/baseline\.(?:json|md)|public-baseline-directional-v1|"
    r"docs/specs/public-memory-benchmark/PRODUCT\.md)",
    re.I,
)


def die(message: str) -> None:
    print(f"FAIL: {message}", file=sys.stderr)
    sys.exit(1)


def load_claim_gate() -> dict[str, str | bool]:
    if not BASELINE_REPORT.is_file():
        die(f"missing baseline report: {BASELINE_REPORT.relative_to(ROOT)}")
    with BASELINE_REPORT.open("r", encoding="utf-8") as handle:
        report = json.load(handle)
    gate = report.get("claim_gate")
    if not isinstance(gate, dict):
        die("baseline report is missing claim_gate")
    return gate


def coding_claim_ready(gate: dict[str, str | bool]) -> bool:
    return (
        gate.get("artifact_verifier_passed") is True
        and gate.get("coding_outcome_stop_loss_status")
        == "ready_for_stop_loss_evaluation"
    )


def sota_claim_ready(gate: dict[str, str | bool]) -> bool:
    status = gate.get("public_sota_status")
    return isinstance(status, str) and not status.startswith("not_")


def line_is_policy_or_negative(text: str) -> bool:
    return CONSERVATIVE_CONTEXT_RE.search(text) is not None


def line_has_report_link(text: str) -> bool:
    return REPORT_LINK_RE.search(text) is not None


def classify_violation(
    text: str, gate: dict[str, str | bool], context: str | None = None
) -> str | None:
    if not STRONG_CLAIM_RE.search(text):
        return None
    context_text = context or text
    if line_is_policy_or_negative(context_text):
        return None

    if SOTA_CLAIM_RE.search(text):
        if sota_claim_ready(gate) and line_has_report_link(text):
            return None
        return "SOTA/best claim lacks a passed Level 3 public claim gate and report link"

    if CODING_CLAIM_RE.search(text):
        if coding_claim_ready(gate) and line_has_report_link(text):
            return None
        return (
            "coding-outcome superiority claim lacks a passed stop-loss gate "
            "and report link"
        )

    return "strong public claim is not grounded in an approved report"


def check_surfaces(gate: dict[str, str | bool]) -> list[str]:
    failures: list[str] = []
    for rel_path in CLAIM_SURFACES:
        path = ROOT / rel_path
        if not path.is_file():
            die(f"missing public claim surface: {rel_path}")
        lines = path.read_text(encoding="utf-8").splitlines()
        for index, line in enumerate(lines):
            context = "\n".join(lines[max(0, index - 2) : index + 2])
            reason = classify_violation(line, gate, context)
            if reason:
                failures.append(f"{rel_path}:{index + 1}: {reason}: {line.strip()}")
    return failures


def run_self_test() -> int:
    blocked_gate = {
        "artifact_verifier_passed": True,
        "coding_outcome_stop_loss_status": "not_evaluated_insufficient_coding_matrix",
        "public_sota_status": "not_evaluated_no_public_sota_claim",
    }
    ready_gate = {
        "artifact_verifier_passed": True,
        "coding_outcome_stop_loss_status": "ready_for_stop_loss_evaluation",
        "public_sota_status": "ready_level3_public_claim",
    }

    cases = [
        (
            "negative SOTA wording passes",
            "README and release wording must not claim SOTA from this report.",
            blocked_gate,
            None,
        ),
        (
            "policy wording passes",
            "Level 2 allowed claim requires the public claim policy gate.",
            blocked_gate,
            None,
        ),
        (
            "unguarded SOTA fails",
            "remem is the best state-of-the-art memory system.",
            blocked_gate,
            "SOTA/best claim",
        ),
        (
            "unguarded coding superiority fails",
            "remem outperforms a maintained context file on coding tasks.",
            blocked_gate,
            "coding-outcome superiority",
        ),
        (
            "ready grounded coding claim passes",
            "remem outperforms no_memory on fixture X; see eval/public/reports/baseline.md.",
            ready_gate,
            None,
        ),
    ]

    for name, text, gate, expected in cases:
        actual = classify_violation(text, gate)
        if expected is None and actual is not None:
            print(f"self-test failed for {name}: {actual}", file=sys.stderr)
            return 1
        if expected is not None and (actual is None or expected not in actual):
            print(f"self-test failed for {name}: {actual!r}", file=sys.stderr)
            return 1
    print("public claims check self-test: ok")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Fail on unsupported strong public benchmark claims."
    )
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()

    if args.self_test:
        return run_self_test()

    gate = load_claim_gate()
    failures = check_surfaces(gate)
    if failures:
        for failure in failures:
            print(f"FAIL: {failure}", file=sys.stderr)
        print(
            "Public claim surfaces may only make SOTA, best, beats, "
            "outperforms, or coding-superiority claims when the relevant "
            "claim gate has passed and the line links to committed report artifacts.",
            file=sys.stderr,
        )
        return 1
    print("public claims check: ok")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
