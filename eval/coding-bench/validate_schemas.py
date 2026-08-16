#!/usr/bin/env python3
"""Offline schema validation for the issue #931 coding-bench harness files.

Validates:
- eval/coding-bench/conditions.json against conditions.schema.json, plus the
  12-enum / 6-stage failure taxonomy and condition id rules;
- eval/coding-bench/examples/curator-log.example.json against
  curator-log.schema.json, plus cross-field budget rules;
- eval/claims/registry.json via the claim gate.
- the three GH931 live-approval schemas are closed, versioned contracts.

Usage: python3 eval/coding-bench/validate_schemas.py
"""

import json
import re
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
RAW_EVENT_KEYS = {
    "event_id",
    "timestamp_epoch",
    "role",
    "sanitized_content",
    "tool_name",
    "sanitized_tool_input",
    "sanitized_tool_output",
    "host_boundary",
}
OPAQUE_EVENT_ID = re.compile(r"^evt-[0-9a-f]{32}$")


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


def check_fixture_raw_events(errors):
    fixture = load(BENCH_DIR / "fixtures" / "tasks.json")
    for task in fixture.get("tasks", []):
        task_id = task.get("id", "<missing>")
        flattened = []
        for episode in task.get("history_episodes", []):
            events = episode.get("raw_events", [])
            if not events:
                errors.append(f"task {task_id}: history episode has no raw_events")
                continue
            flattened.extend(events)
            for memory in episode.get("memories", []):
                text = memory.get("text")
                if not text or not any(text in event.get("sanitized_content", "") for event in events):
                    errors.append(
                        f"task {task_id}: memory evidence exists only in gold fields, not raw_events"
                    )
        ids = [event.get("event_id") for event in flattened]
        if len(ids) != len(set(ids)):
            errors.append(f"task {task_id}: duplicate raw event id")
        previous_timestamp = None
        for event in flattened:
            if set(event) != RAW_EVENT_KEYS:
                errors.append(f"task {task_id}: raw event fields are not the closed v1 set")
                continue
            event_id = event["event_id"]
            if not isinstance(event_id, str) or not OPAQUE_EVENT_ID.fullmatch(event_id):
                errors.append(f"task {task_id}: invalid opaque raw event id")
            timestamp = event["timestamp_epoch"]
            if not isinstance(timestamp, int) or timestamp <= 0:
                errors.append(f"task {task_id}: invalid raw event timestamp")
            elif previous_timestamp is not None and timestamp < previous_timestamp:
                errors.append(f"task {task_id}: raw event timestamps decrease in source order")
            previous_timestamp = timestamp
            boundary = event["host_boundary"]
            role = event["role"]
            content = event["sanitized_content"]
            tool_name = event["tool_name"]
            tool_input = event["sanitized_tool_input"]
            tool_output = event["sanitized_tool_output"]
            valid = {
                "user_message": role == "user" and bool(content) and tool_name is None and tool_input is None and tool_output is None,
                "assistant_message": role == "assistant" and bool(content) and tool_name is None and tool_input is None and tool_output is None,
                "tool_call": role == "assistant" and bool(tool_name) and bool(tool_input) and tool_output is None,
                "tool_result": role == "tool" and bool(tool_name) and bool(tool_output) and tool_input is None,
            }.get(boundary, False)
            if not valid:
                errors.append(f"task {task_id}: invalid raw event role/tool/boundary shape")
        supporting = task.get("gold_memory", {}).get("supporting_event_ids", [])
        if not set(supporting).issubset(set(ids)):
            errors.append(f"task {task_id}: gold supporting event is absent from raw_events")


def check_live_approval_schemas(errors):
    schema_names = (
        "live-approval-trust-root.schema.json",
        "live-approval.schema.json",
        "supervisor-attestation.schema.json",
    )
    for name in schema_names:
        schema = load(BENCH_DIR / "schemas" / name)
        if schema.get("$schema") != "https://json-schema.org/draft/2020-12/schema":
            errors.append(f"{name}: must use JSON Schema draft 2020-12")
        if schema.get("additionalProperties") is not False:
            errors.append(f"{name}: root object must be closed")
        if schema.get("properties", {}).get("schema_version") != {"const": 1}:
            errors.append(f"{name}: must pin schema_version to 1")
        required = set(schema.get("required", []))
        if required != {"schema_version", "payload", "signature"} and name != "live-approval-trust-root.schema.json":
            errors.append(f"{name}: signed envelope fields are not the closed v1 set")


def main():
    errors = []
    check_conditions(errors)
    check_curator_log(errors)
    check_fixture_raw_events(errors)
    check_live_approval_schemas(errors)
    claim_errors = claim_gate.check(REPO_ROOT / "eval" / "claims" / "registry.json")
    errors.extend(f"claim gate: {e}" for e in claim_errors)
    if errors:
        for error in errors:
            print(f"FAIL {error}")
        return 1
    print("OK coding-bench schemas, raw-event fixture, curator log, and claims registry validate")
    return 0


if __name__ == "__main__":
    sys.exit(main())
