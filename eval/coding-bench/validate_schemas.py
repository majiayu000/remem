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
        errors.append("legacy condition id 'remem' must not appear; use remem_seeded_sessionstart")
    if "remem_preloaded" in all_ids:
        errors.append("historical condition id 'remem_preloaded' must not describe current runner semantics")
    if "curated_file" in all_ids:
        errors.append("legacy condition id 'curated_file' must not appear; use curated_file_expert")
    for cond in registry["primary_conditions"] + registry["diagnostic_conditions"]:
        if "current_runner_id" in cond:
            errors.append(f"condition {cond['id']}: current_runner_id aliases are forbidden")
        if cond["id"] in {"remem_preloaded", "curated_file_expert"}:
            if cond["runner_status"] != "implemented":
                errors.append(f"condition {cond['id']}: renamed diagnostic id must be implemented")

    public_run_schema = load(REPO_ROOT / "eval" / "public" / "schemas" / "coding-run.schema.json")
    public_report_schema = load(
        REPO_ROOT / "eval" / "public" / "schemas" / "coding-report.schema.json"
    )
    identity_fields = {"benchmark_id", "benchmark_version", "run_phase", "matrix_namespace"}
    if not identity_fields.issubset(public_run_schema["required"]):
        errors.append("public coding-run schema must require canonical benchmark identity")
    if not identity_fields.issubset(public_report_schema["required"]):
        errors.append("public coding-report schema must require canonical benchmark identity")
    public_ids = public_run_schema["properties"]["condition"]["enum"]
    expected_public_ids = set(all_ids) | {"remem_preloaded"}
    if set(public_ids) != expected_public_ids or len(public_ids) != len(set(public_ids)):
        errors.append(
            "public coding-run condition enum must match the machine registry plus "
            "historical remem_preloaded"
        )
    audited_public_ids = set(
        public_run_schema["allOf"][0]["if"]["properties"]["condition"]["enum"]
    )
    expected_audited_ids = {condition for condition in all_ids if condition.startswith("remem_")}
    if audited_public_ids != expected_audited_ids:
        errors.append("public coding-run audited condition set must match current remem conditions")
    audited_required = set(public_run_schema["allOf"][0]["then"]["required"])
    if "injected_context_sha256" not in audited_required:
        errors.append("current remem coding runs must bind exact injected-context bytes")
    remem_contract_ids = set(
        public_run_schema["allOf"][1]["if"]["properties"]["condition"]["enum"]
    )
    expected_remem_contract_ids = {
        condition for condition in expected_public_ids if condition.startswith("remem_")
    }
    if remem_contract_ids != expected_remem_contract_ids:
        errors.append("public coding-run memory-contract set must match remem conditions")
    control_ids = set(
        public_run_schema["allOf"][2]["if"]["properties"]["condition"]["enum"]
    )
    expected_control_ids = {condition for condition in all_ids if not condition.startswith("remem_")}
    if control_ids != expected_control_ids:
        errors.append("public coding-run control set must match current non-remem conditions")
    historical_audit_rule = public_run_schema["allOf"][3]
    if (
        historical_audit_rule["if"]["properties"]["condition"] != {"const": "remem_preloaded"}
        or historical_audit_rule["then"]["properties"]
        != {
            "context_audit_status": {"type": "null"},
            "context_audit_failure_reason": {"type": "null"},
            "remem_context_audit": {"type": "null"},
            "injected_context_sha256": {"type": "null"},
        }
    ):
        errors.append("historical remem_preloaded must not accept current ContextAudit evidence")

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
