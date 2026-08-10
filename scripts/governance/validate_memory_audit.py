#!/usr/bin/env python3
"""Fail-closed semantic validator for memory-audit-run v1 artifacts."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
from pathlib import Path
from typing import Any


VERDICTS = {
    "keep",
    "stale",
    "split",
    "merge_candidate",
    "quarantine",
    "abstain",
}
SHA256_RE = re.compile(r"^[0-9a-f]{64}$")
COMMIT_RE = re.compile(r"^[0-9a-f]{40}([0-9a-f]{24})?$")
RUN_ID_RE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$")
REASON_RE = re.compile(r"^[a-z][a-z0-9_]{0,63}$")
EVIDENCE_RE = re.compile(
    r"^(captured_event|observation|memory|commit|file|user_prompt):[^\s]{1,240}$"
)

TOP_FIELDS = {"schema_version", "run_id", "source", "model", "policy", "records"}
SOURCE_FIELDS = {
    "database_snapshot_sha256",
    "selected_ids_sha256",
    "source_code_commit",
    "runtime_version",
    "database_schema_version",
    "content_redacted",
}
MODEL_FIELDS = {
    "provider",
    "model_id",
    "prompt_protocol_version",
    "input_schema_version",
}
POLICY_FIELDS = {"confidence_threshold", "max_batch_size", "mutation_authorized"}
RECORD_FIELDS = {
    "memory_id",
    "row_snapshot_sha256",
    "verdict",
    "confidence",
    "reason_codes",
    "evidence_refs",
}


class AuditValidationError(ValueError):
    pass


def fail(message: str) -> None:
    raise AuditValidationError(message)


def require_object(value: Any, path: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        fail(f"{path} must be an object")
    return value


def require_exact_fields(value: dict[str, Any], expected: set[str], path: str) -> None:
    missing = sorted(expected - value.keys())
    extra = sorted(value.keys() - expected)
    if missing:
        fail(f"{path} missing fields: {', '.join(missing)}")
    if extra:
        fail(f"{path} has unsupported fields: {', '.join(extra)}")


def require_string(value: Any, path: str, minimum: int = 1, maximum: int = 256) -> str:
    if not isinstance(value, str) or not minimum <= len(value) <= maximum:
        fail(f"{path} must be a string with length {minimum}..{maximum}")
    return value


def require_integer(value: Any, path: str, minimum: int, maximum: int | None = None) -> int:
    if isinstance(value, bool) or not isinstance(value, int) or value < minimum:
        fail(f"{path} must be an integer >= {minimum}")
    if maximum is not None and value > maximum:
        fail(f"{path} must be an integer <= {maximum}")
    return value


def require_probability(value: Any, path: str) -> float:
    if isinstance(value, bool) or not isinstance(value, (int, float)) or not 0 <= value <= 1:
        fail(f"{path} must be a number in [0, 1]")
    return float(value)


def require_pattern(value: Any, pattern: re.Pattern[str], path: str) -> str:
    text = require_string(value, path)
    if pattern.fullmatch(text) is None:
        fail(f"{path} has invalid format")
    return text


def selected_ids_sha256(ids: list[int]) -> str:
    payload = json.dumps(ids, separators=(",", ":")).encode("utf-8")
    return hashlib.sha256(payload).hexdigest()


def validate_artifact(value: Any) -> dict[str, Any]:
    artifact = require_object(value, "artifact")
    require_exact_fields(artifact, TOP_FIELDS, "artifact")
    if artifact["schema_version"] != 1:
        fail("schema_version must equal 1")
    require_pattern(artifact["run_id"], RUN_ID_RE, "run_id")

    source = require_object(artifact["source"], "source")
    require_exact_fields(source, SOURCE_FIELDS, "source")
    require_pattern(source["database_snapshot_sha256"], SHA256_RE, "source.database_snapshot_sha256")
    expected_ids_digest = require_pattern(
        source["selected_ids_sha256"], SHA256_RE, "source.selected_ids_sha256"
    )
    require_pattern(source["source_code_commit"], COMMIT_RE, "source.source_code_commit")
    require_string(source["runtime_version"], "source.runtime_version", maximum=128)
    require_integer(source["database_schema_version"], "source.database_schema_version", 1)
    if source["content_redacted"] is not True:
        fail("source.content_redacted must be true")

    model = require_object(artifact["model"], "model")
    require_exact_fields(model, MODEL_FIELDS, "model")
    require_string(model["provider"], "model.provider", maximum=128)
    require_string(model["model_id"], "model.model_id", maximum=256)
    require_string(model["prompt_protocol_version"], "model.prompt_protocol_version", maximum=128)
    require_integer(model["input_schema_version"], "model.input_schema_version", 1)

    policy = require_object(artifact["policy"], "policy")
    require_exact_fields(policy, POLICY_FIELDS, "policy")
    threshold = require_probability(policy["confidence_threshold"], "policy.confidence_threshold")
    max_batch_size = require_integer(policy["max_batch_size"], "policy.max_batch_size", 1, 1000)
    if policy["mutation_authorized"] is not False:
        fail("policy.mutation_authorized must be false")

    records = artifact["records"]
    if not isinstance(records, list):
        fail("records must be an array")
    if len(records) > max_batch_size:
        fail("records exceeds policy.max_batch_size")

    ids: list[int] = []
    previous_id = 0
    for index, raw_record in enumerate(records):
        path = f"records[{index}]"
        record = require_object(raw_record, path)
        require_exact_fields(record, RECORD_FIELDS, path)
        memory_id = require_integer(record["memory_id"], f"{path}.memory_id", 1)
        if memory_id <= previous_id:
            fail("records must be sorted by memory_id with no duplicate IDs")
        previous_id = memory_id
        ids.append(memory_id)
        require_pattern(record["row_snapshot_sha256"], SHA256_RE, f"{path}.row_snapshot_sha256")
        verdict = record["verdict"]
        if verdict not in VERDICTS:
            fail(f"{path}.verdict is not allowed")
        confidence = require_probability(record["confidence"], f"{path}.confidence")

        reasons = record["reason_codes"]
        if not isinstance(reasons, list) or not reasons:
            fail(f"{path}.reason_codes must be a non-empty array")
        if len(reasons) != len(set(reasons)):
            fail(f"{path}.reason_codes must be unique")
        for reason_index, reason in enumerate(reasons):
            require_pattern(reason, REASON_RE, f"{path}.reason_codes[{reason_index}]")

        evidence = record["evidence_refs"]
        if not isinstance(evidence, list):
            fail(f"{path}.evidence_refs must be an array")
        if len(evidence) != len(set(evidence)):
            fail(f"{path}.evidence_refs must be unique")
        for evidence_index, evidence_ref in enumerate(evidence):
            require_pattern(
                evidence_ref, EVIDENCE_RE, f"{path}.evidence_refs[{evidence_index}]"
            )

        if confidence < threshold and verdict != "abstain":
            fail(f"{path} is below confidence threshold and must abstain")
        if verdict != "abstain" and not evidence:
            fail(f"{path} requires evidence_refs for verdict {verdict}")

    actual_ids_digest = selected_ids_sha256(ids)
    if expected_ids_digest != actual_ids_digest:
        fail(
            "source.selected_ids_sha256 does not match the sorted record IDs "
            f"(expected {actual_ids_digest})"
        )
    return artifact


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="validate a remem memory-audit-run v1 artifact")
    parser.add_argument("artifact", type=Path)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    try:
        value = json.loads(args.artifact.read_text(encoding="utf-8"))
        validate_artifact(value)
    except (OSError, json.JSONDecodeError, AuditValidationError) as error:
        print(f"invalid: {error}", file=sys.stderr)
        return 2
    print("valid")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
