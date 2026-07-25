#!/usr/bin/env python3
"""Offline schema validation for the issue #931 coding-bench harness files.

Validates:
- eval/coding-bench/conditions.json against conditions.schema.json, plus the
  12-enum / 6-stage failure taxonomy and condition id rules;
- eval/coding-bench/examples/curator-log.example.json against
  curator-log.schema.json, plus cross-field budget rules;
- eval/claims/registry.json via the claim gate.

Usage: python3 eval/coding-bench/validate_schemas.py
"""

import json
import sys
from pathlib import Path

BENCH_DIR = Path(__file__).resolve().parent
REPO_ROOT = BENCH_DIR.parent.parent
sys.path.insert(0, str(REPO_ROOT / "eval" / "claims"))

import claim_gate  # noqa: E402

EXPECTED_FAILURE_ENUMS = {
    "evidence_not_captured",
    "durable_fact_missed",
    "unsupported_claim_saved",
    "wrong_scope",
    "update_not_applied",
    "conflict_not_detected",
    "stale_memory_not_invalidated",
    "relevant_memory_missing",
    "irrelevant_memory_selected",
    "context_budget_dropped",
    "retrieved_but_ignored",
    "memory_misapplied",
}
EXPECTED_PRIMARY = ["no_memory", "curated_file_budgeted", "remem_e2e"]


def load(path):
    return json.loads(path.read_text())


def check_conditions(errors):
    registry = load(BENCH_DIR / "conditions.json")
    schema = load(BENCH_DIR / "schemas" / "conditions.schema.json")
    errors.extend(claim_gate.validate_schema(registry, schema))

    primary_ids = [c["id"] for c in registry["primary_conditions"]]
    if primary_ids != EXPECTED_PRIMARY:
        errors.append(f"primary condition ids must be {EXPECTED_PRIMARY}, got {primary_ids}")
    all_ids = primary_ids + [c["id"] for c in registry["diagnostic_conditions"]]
    if len(all_ids) != len(set(all_ids)):
        errors.append("duplicate condition ids")
    if "remem" in all_ids:
        errors.append("legacy condition id 'remem' must not appear; use remem_preloaded")
    if "curated_file" in all_ids:
        errors.append("legacy condition id 'curated_file' must not appear; use curated_file_expert")

    seen_enums = []
    for stage, enums in registry["failure_stages"].items():
        seen_enums.extend(enums)
    if sorted(seen_enums) != sorted(EXPECTED_FAILURE_ENUMS):
        missing = EXPECTED_FAILURE_ENUMS - set(seen_enums)
        extra = set(seen_enums) - EXPECTED_FAILURE_ENUMS
        errors.append(f"failure taxonomy mismatch: missing={sorted(missing)} extra={sorted(extra)}")
    if len(seen_enums) != len(set(seen_enums)):
        errors.append("failure enum mapped to more than one stage")

    for ref_key in ("registry", "gate_script"):
        ref = REPO_ROOT / registry["claim_gate"][ref_key]
        if not ref.is_file():
            errors.append(f"claim_gate.{ref_key} missing on disk: {ref}")
    for cond in registry["primary_conditions"] + registry["diagnostic_conditions"]:
        for key in ("protocol", "artifact_schema"):
            if key in cond and not (REPO_ROOT / cond[key]).is_file():
                errors.append(f"condition {cond['id']}: {key} missing on disk: {cond[key]}")


def check_curator_log(errors):
    log = load(BENCH_DIR / "examples" / "curator-log.example.json")
    schema = load(BENCH_DIR / "schemas" / "curator-log.schema.json")
    errors.extend(claim_gate.validate_schema(log, schema))
    if errors:
        return
    budget = log["budget"]
    sums = {"minutes": 0.0, "edits": 0, "deletions": 0, "conflicts": 0}
    for i, session in enumerate(log["sessions"]):
        if session["minutes_spent"] > budget["minutes_per_session"]:
            errors.append(f"sessions[{i}]: minutes_spent exceeds budget")
        if session["chars_after"] > budget["max_chars"]:
            errors.append(f"sessions[{i}]: chars_after exceeds max_chars")
        sums["minutes"] += session["minutes_spent"]
        sums["edits"] += session["edit_count"]
        sums["deletions"] += session["deletion_count"]
        sums["conflicts"] += session["conflict_resolution_count"]
    totals = log["totals"]
    if log["final_char_count"] > budget["max_chars"]:
        errors.append("final_char_count exceeds max_chars")
    if abs(totals["maintenance_minutes"] - sums["minutes"]) > 1e-9:
        errors.append("totals.maintenance_minutes does not match session sum")
    if totals["update_count"] != sums["edits"]:
        errors.append("totals.update_count does not match session sum")
    if totals["deletion_count"] != sums["deletions"]:
        errors.append("totals.deletion_count does not match session sum")
    if totals["conflict_resolution_count"] != sums["conflicts"]:
        errors.append("totals.conflict_resolution_count does not match session sum")


def main():
    errors = []
    check_conditions(errors)
    check_curator_log(errors)
    claim_errors = claim_gate.check(REPO_ROOT / "eval" / "claims" / "registry.json")
    errors.extend(f"claim gate: {e}" for e in claim_errors)
    if errors:
        for error in errors:
            print(f"FAIL {error}")
        return 1
    print("OK conditions.json, curator-log example, and claims registry validate")
    return 0


if __name__ == "__main__":
    sys.exit(main())
