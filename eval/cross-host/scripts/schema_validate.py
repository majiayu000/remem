#!/usr/bin/env python3
"""Stdlib-only validator for the JSON Schema subset used by eval/cross-host.

Supported keywords: type, const, enum, required, properties,
additionalProperties (boolean), items, minItems, minLength, pattern,
minimum, maximum, uniqueItems. Unknown keywords raise instead of being
silently ignored so the schemas cannot drift ahead of the validator.
"""

from __future__ import annotations

import json
import math
import re
import sys
from decimal import Decimal
from pathlib import Path

IGNORED_KEYWORDS = {"$schema", "$id", "title", "description"}
SUPPORTED_KEYWORDS = {
    "type",
    "const",
    "enum",
    "required",
    "properties",
    "additionalProperties",
    "items",
    "minItems",
    "minLength",
    "pattern",
    "minimum",
    "maximum",
    "uniqueItems",
}

TYPE_CHECKS = {
    "object": lambda v: isinstance(v, dict),
    "array": lambda v: isinstance(v, list),
    "string": lambda v: isinstance(v, str),
    "integer": lambda v: isinstance(v, int) and not isinstance(v, bool),
    "number": lambda v: isinstance(v, (int, float)) and not isinstance(v, bool),
    "boolean": lambda v: isinstance(v, bool),
    "null": lambda v: v is None,
}
JSON_SAFE_INTEGER_MAX = (1 << 53) - 1


def reject_duplicate_keys(pairs: list[tuple[str, object]]) -> dict:
    value = {}
    for key, item in pairs:
        if key in value:
            raise ValueError(f"duplicate object key {key!r}")
        value[key] = item
    return value


def reject_non_finite_number(value: str) -> object:
    raise ValueError(f"non-finite number {value!r}")


def parse_finite_float(value: str) -> float:
    """Parse a JSON float literal, rejecting the two silent binary64 edges.

    Overflow already surfaces as an infinity, but underflow does not: a literal
    like ``1e-400`` converts to ``0.0``, which would turn a nonzero measurement
    into a zero that passes every finiteness check downstream.
    """
    parsed = float(value)
    if not math.isfinite(parsed):
        raise ValueError(f"non-finite number {value!r}")
    if parsed == 0.0 and Decimal(value) != 0:
        raise ValueError(f"number {value!r} underflows to zero")
    return parsed


def parse_safe_integer(value: str) -> int:
    parsed = int(value)
    if not -JSON_SAFE_INTEGER_MAX <= parsed <= JSON_SAFE_INTEGER_MAX:
        raise ValueError(f"integer {value!r} is outside the safe JSON range")
    return parsed


def reject_unpaired_surrogates(value: object, path: str = "$") -> None:
    """Reject decoded strings that are not valid Unicode scalar sequences.

    ``json.loads`` accepts escaped lone surrogates such as ``"\\ud800"``, which
    later make UTF-8 encoding and RFC 8785 canonicalization fail or disagree.
    """
    if isinstance(value, str):
        try:
            value.encode("utf-8")
        except UnicodeEncodeError as exc:
            raise ValueError(f"{path}: string contains an unpaired surrogate") from exc
    elif isinstance(value, list):
        for index, item in enumerate(value):
            reject_unpaired_surrogates(item, f"{path}[{index}]")
    elif isinstance(value, dict):
        for key, item in value.items():
            try:
                key.encode("utf-8")
            except UnicodeEncodeError as exc:
                raise ValueError(
                    f"{path}: object key {key!a} contains an unpaired surrogate"
                ) from exc
            reject_unpaired_surrogates(item, f"{path}.{key}")


def parse_json(value: str) -> object:
    parsed = json.loads(
        value,
        object_pairs_hook=reject_duplicate_keys,
        parse_constant=reject_non_finite_number,
        parse_float=parse_finite_float,
        parse_int=parse_safe_integer,
    )
    reject_unpaired_surrogates(parsed)
    return parsed


def load_json(path: Path) -> object:
    try:
        return parse_json(path.read_text(encoding="utf-8"))
    except (json.JSONDecodeError, UnicodeError, ValueError) as exc:
        raise ValueError(f"{path}: invalid JSON: {exc}") from exc


def _check_type(value: object, expected: object, path: str, errors: list[str]) -> None:
    types = expected if isinstance(expected, list) else [expected]
    if not any(TYPE_CHECKS[t](value) for t in types):
        errors.append(f"{path}: expected type {types}, got {type(value).__name__}")


def validate(value: object, schema: dict, path: str = "$") -> list[str]:
    errors: list[str] = []
    unknown = set(schema) - SUPPORTED_KEYWORDS - IGNORED_KEYWORDS
    if unknown:
        raise ValueError(f"{path}: unsupported schema keywords {sorted(unknown)}")

    if "const" in schema and value != schema["const"]:
        errors.append(f"{path}: expected const {schema['const']!r}, got {value!r}")
    if "enum" in schema and value not in schema["enum"]:
        errors.append(f"{path}: value {value!r} not in enum")
    if "type" in schema:
        _check_type(value, schema["type"], path, errors)
        if errors:
            return errors

    if isinstance(value, str):
        if "minLength" in schema and len(value) < schema["minLength"]:
            errors.append(f"{path}: string shorter than minLength {schema['minLength']}")
        if "pattern" in schema and not re.search(schema["pattern"], value):
            errors.append(f"{path}: string does not match pattern {schema['pattern']!r}")

    if isinstance(value, (int, float)) and not isinstance(value, bool):
        if "minimum" in schema and value < schema["minimum"]:
            errors.append(f"{path}: {value} below minimum {schema['minimum']}")
        if "maximum" in schema and value > schema["maximum"]:
            errors.append(f"{path}: {value} above maximum {schema['maximum']}")

    if isinstance(value, list):
        if "minItems" in schema and len(value) < schema["minItems"]:
            errors.append(f"{path}: fewer than minItems {schema['minItems']}")
        if schema.get("uniqueItems"):
            seen = [json.dumps(v, sort_keys=True) for v in value]
            if len(seen) != len(set(seen)):
                errors.append(f"{path}: items are not unique")
        if "items" in schema:
            for i, item in enumerate(value):
                errors.extend(validate(item, schema["items"], f"{path}[{i}]"))

    if isinstance(value, dict):
        for key in schema.get("required", []):
            if key not in value:
                errors.append(f"{path}: missing required key {key!r}")
        props = schema.get("properties", {})
        for key, sub in props.items():
            if key in value:
                errors.extend(validate(value[key], sub, f"{path}.{key}"))
        if schema.get("additionalProperties") is False:
            extra = set(value) - set(props)
            if extra:
                errors.append(f"{path}: unexpected keys {sorted(extra)}")

    return errors


def validate_file(data_path: Path, schema_path: Path) -> list[str]:
    schema = load_json(schema_path)
    data = load_json(data_path)
    if not isinstance(schema, dict):
        raise ValueError(f"{schema_path}: schema must be a JSON object")
    return validate(data, schema)


def self_test() -> int:
    schema = {
        "type": "object",
        "required": ["id", "kind", "count", "tags"],
        "additionalProperties": False,
        "properties": {
            "id": {"type": "string", "minLength": 1, "pattern": "^[a-z-]+$"},
            "kind": {"enum": ["a", "b"]},
            "count": {"type": "integer", "minimum": 0, "maximum": 10},
            "tags": {
                "type": "array",
                "minItems": 1,
                "uniqueItems": True,
                "items": {"type": "string", "minLength": 1},
            },
            "note": {"type": ["string", "null"]},
        },
    }
    good = {"id": "ok-id", "kind": "a", "count": 3, "tags": ["x"], "note": None}
    cases = [
        ("valid object passes", good, 0),
        ("missing required fails", {"id": "x", "kind": "a", "count": 1}, 1),
        ("bad enum fails", {**good, "kind": "z"}, 1),
        ("bad pattern fails", {**good, "id": "BAD"}, 1),
        ("bool is not integer", {**good, "count": True}, 1),
        ("above maximum fails", {**good, "count": 11}, 1),
        ("duplicate items fail", {**good, "tags": ["x", "x"]}, 1),
        ("empty array fails minItems", {**good, "tags": []}, 1),
        ("extra key fails", {**good, "zzz": 1}, 1),
        ("nullable accepts string", {**good, "note": "hi"}, 0),
    ]
    failures = 0
    for name, value, expected_errors in cases:
        errors = validate(value, schema)
        ok = (len(errors) == 0) == (expected_errors == 0)
        print(f"{'PASS' if ok else 'FAIL'}: {name} ({len(errors)} errors)")
        if not ok:
            failures += 1
            for err in errors:
                print(f"  {err}")
    try:
        validate({}, {"type": "object", "oneOf": []})
        print("FAIL: unsupported keyword not rejected")
        failures += 1
    except ValueError:
        print("PASS: unsupported keyword rejected")
    print(f"self-test: {failures} failures")
    return 1 if failures else 0


def main(argv: list[str]) -> int:
    if len(argv) == 2 and argv[1] == "--self-test":
        return self_test()
    if len(argv) != 3:
        print("usage: schema_validate.py <data.json> <schema.json> | --self-test")
        return 2
    errors = validate_file(Path(argv[1]), Path(argv[2]))
    for err in errors:
        print(err)
    print(f"{argv[1]}: {'PASS' if not errors else f'FAIL ({len(errors)} errors)'}")
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
