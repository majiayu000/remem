"""Closed JSON Schema 2020-12 contract for remem's published schemas."""

from __future__ import annotations

import math
import re
from pathlib import Path
from typing import Any

from specrail_lib import SUPPORTED_SCHEMA_KEYS


JSON_TYPES = frozenset(
    {"array", "boolean", "integer", "null", "number", "object", "string"}
)

# Closed, documented JSON Schema 2020-12 vocabulary adopted by this repository.
# Keys in properties/$defs/patternProperties/dependent* maps are names, not schema
# keywords; only the mapped values are recursively validated as schema nodes.
ADOPTED_SCHEMA_KEYWORDS = frozenset(
    {
        "$anchor",
        "$comment",
        "$defs",
        "$dynamicAnchor",
        "$dynamicRef",
        "$id",
        "$ref",
        "$schema",
        "$vocabulary",
        "additionalProperties",
        "allOf",
        "anyOf",
        "const",
        "contains",
        "contentEncoding",
        "contentMediaType",
        "contentSchema",
        "default",
        "dependentRequired",
        "dependentSchemas",
        "deprecated",
        "description",
        "else",
        "enum",
        "examples",
        "exclusiveMaximum",
        "exclusiveMinimum",
        "format",
        "if",
        "items",
        "maxContains",
        "maxItems",
        "maxLength",
        "maxProperties",
        "maximum",
        "minContains",
        "minItems",
        "minLength",
        "minProperties",
        "minimum",
        "multipleOf",
        "not",
        "oneOf",
        "pattern",
        "patternProperties",
        "prefixItems",
        "properties",
        "propertyNames",
        "readOnly",
        "required",
        "then",
        "title",
        "type",
        "unevaluatedItems",
        "unevaluatedProperties",
        "uniqueItems",
        "writeOnly",
    }
)

RUNTIME_PROFILE_SCHEMAS = frozenset({"duplicate_work_evidence.schema.json"})

SCHEMA_MAP_KEYWORDS = ("$defs", "dependentSchemas", "patternProperties", "properties")
SCHEMA_LIST_KEYWORDS = ("allOf", "anyOf", "oneOf", "prefixItems")
SCHEMA_NODE_KEYWORDS = ("contains", "contentSchema", "else", "if", "not", "propertyNames", "then")
BOOLEAN_OR_SCHEMA_KEYWORDS = (
    "additionalProperties",
    "unevaluatedItems",
    "unevaluatedProperties",
)

STRING_KEYWORDS = (
    "$anchor",
    "$comment",
    "$dynamicAnchor",
    "$dynamicRef",
    "$id",
    "$ref",
    "$schema",
    "contentEncoding",
    "contentMediaType",
    "description",
    "format",
    "title",
)
BOOLEAN_KEYWORDS = ("deprecated", "readOnly", "uniqueItems", "writeOnly")
NON_NEGATIVE_INTEGER_KEYWORDS = (
    "maxContains",
    "maxItems",
    "maxLength",
    "maxProperties",
    "minContains",
    "minItems",
    "minLength",
    "minProperties",
)
NUMBER_KEYWORDS = ("exclusiveMaximum", "exclusiveMinimum", "maximum", "minimum")

KEYWORD_APPLICABLE_TYPES = {
    "additionalProperties": {"object"},
    "dependentRequired": {"object"},
    "dependentSchemas": {"object"},
    "items": {"array"},
    "maxContains": {"array"},
    "maxItems": {"array"},
    "maxLength": {"string"},
    "maxProperties": {"object"},
    "maximum": {"integer", "number"},
    "minContains": {"array"},
    "minItems": {"array"},
    "minLength": {"string"},
    "minProperties": {"object"},
    "minimum": {"integer", "number"},
    "multipleOf": {"integer", "number"},
    "pattern": {"string"},
    "patternProperties": {"object"},
    "prefixItems": {"array"},
    "properties": {"object"},
    "propertyNames": {"object"},
    "required": {"object"},
    "uniqueItems": {"array"},
    "contains": {"array"},
    "exclusiveMaximum": {"integer", "number"},
    "exclusiveMinimum": {"integer", "number"},
}

RUNTIME_TYPED_KEYWORDS = {
    "exclusiveMaximum": {"integer", "number"},
    "exclusiveMinimum": {"integer", "number"},
    "items": {"array"},
    "minItems": {"array"},
    "minLength": {"string"},
    "minimum": {"integer", "number"},
    "required": {"object"},
}


def uses_runtime_profile(path: Path) -> bool:
    return path.name in RUNTIME_PROFILE_SCHEMAS


def _is_json_number(value: object) -> bool:
    return (
        isinstance(value, (int, float))
        and not isinstance(value, bool)
        and (not isinstance(value, float) or math.isfinite(value))
    )


def _compile_pattern(
    value: object,
    relative_path: Path,
    keyword_path: str,
    errors: list[str],
) -> None:
    if not isinstance(value, str):
        errors.append(f"{relative_path}: {keyword_path} must be a string")
        return
    try:
        re.compile(value)
    except re.error as exc:
        errors.append(f"{relative_path}: {keyword_path} invalid regex: {exc.msg}")


def _declared_types(
    schema: dict[str, Any],
    relative_path: Path,
    schema_path: str,
    errors: list[str],
) -> set[str] | None:
    if "type" not in schema:
        return None
    value = schema["type"]
    values = value if isinstance(value, list) else [value]
    if (
        not values
        or not all(isinstance(item, str) for item in values)
        or not set(values) <= JSON_TYPES
        or len(set(values)) != len(values)
    ):
        errors.append(
            f"{relative_path}: {schema_path}.type must be a supported JSON type "
            "or non-empty unique array of supported JSON types"
        )
        return None
    return set(values)


def _validate_shapes(
    schema: dict[str, Any],
    relative_path: Path,
    schema_path: str,
    errors: list[str],
) -> set[str] | None:
    declared_types = _declared_types(schema, relative_path, schema_path, errors)

    for keyword in STRING_KEYWORDS:
        if keyword in schema and not isinstance(schema[keyword], str):
            errors.append(f"{relative_path}: {schema_path}.{keyword} must be a string")
    for keyword in BOOLEAN_KEYWORDS:
        if keyword in schema and not isinstance(schema[keyword], bool):
            errors.append(f"{relative_path}: {schema_path}.{keyword} must be a boolean")
    for keyword in NON_NEGATIVE_INTEGER_KEYWORDS:
        value = schema.get(keyword)
        if keyword in schema and (
            not isinstance(value, int) or isinstance(value, bool) or value < 0
        ):
            errors.append(
                f"{relative_path}: {schema_path}.{keyword} must be a non-negative integer"
            )
    for keyword in NUMBER_KEYWORDS:
        if keyword in schema and not _is_json_number(schema[keyword]):
            errors.append(f"{relative_path}: {schema_path}.{keyword} must be a JSON number")
    if "multipleOf" in schema and (
        not _is_json_number(schema["multipleOf"]) or schema["multipleOf"] <= 0
    ):
        errors.append(f"{relative_path}: {schema_path}.multipleOf must be a positive JSON number")
    if "enum" in schema and (
        not isinstance(schema["enum"], list) or not schema["enum"]
    ):
        errors.append(f"{relative_path}: {schema_path}.enum must be a non-empty array")
    if "examples" in schema and not isinstance(schema["examples"], list):
        errors.append(f"{relative_path}: {schema_path}.examples must be an array")
    if "$vocabulary" in schema:
        vocabulary = schema["$vocabulary"]
        if not isinstance(vocabulary, dict) or not all(
            isinstance(uri, str) and isinstance(required, bool)
            for uri, required in vocabulary.items()
        ):
            errors.append(
                f"{relative_path}: {schema_path}.$vocabulary must be an object of booleans"
            )
    if "required" in schema:
        required = schema["required"]
        if (
            not isinstance(required, list)
            or not all(isinstance(name, str) for name in required)
            or len(set(required)) != len(required)
        ):
            errors.append(
                f"{relative_path}: {schema_path}.required must be a unique array of strings"
            )
    if "dependentRequired" in schema:
        dependencies = schema["dependentRequired"]
        if not isinstance(dependencies, dict):
            errors.append(
                f"{relative_path}: {schema_path}.dependentRequired must be an object"
            )
        else:
            for name, required_names in dependencies.items():
                if (
                    not isinstance(required_names, list)
                    or not all(isinstance(item, str) for item in required_names)
                    or len(set(required_names)) != len(required_names)
                ):
                    errors.append(
                        f"{relative_path}: {schema_path}.dependentRequired.{name} "
                        "must be a unique array of strings"
                    )
    if "pattern" in schema:
        _compile_pattern(schema["pattern"], relative_path, f"{schema_path}.pattern", errors)
    return declared_types


def _validate_applicability(
    schema: dict[str, Any],
    declared_types: set[str] | None,
    relative_path: Path,
    schema_path: str,
    errors: list[str],
) -> None:
    if declared_types is None:
        return
    for keyword, applicable_types in KEYWORD_APPLICABLE_TYPES.items():
        if keyword in schema and declared_types.isdisjoint(applicable_types):
            declared = " or ".join(sorted(declared_types))
            applicable = " or ".join(sorted(applicable_types))
            errors.append(
                f"{relative_path}: {schema_path}.{keyword} is incompatible with "
                f"declared type {declared}; applicable type is {applicable}"
            )


def _validate_runtime_profile(
    schema: dict[str, Any],
    declared_types: set[str] | None,
    relative_path: Path,
    schema_path: str,
    errors: list[str],
) -> None:
    for keyword in sorted(set(schema) - SUPPORTED_SCHEMA_KEYS):
        if keyword in ADOPTED_SCHEMA_KEYWORDS:
            errors.append(
                f"{relative_path}: {schema_path}: runtime profile does not support "
                f"JSON Schema keyword {keyword!r}"
            )
    for keyword, compatible_types in RUNTIME_TYPED_KEYWORDS.items():
        if keyword not in schema:
            continue
        expected = " or ".join(sorted(compatible_types))
        if declared_types is None:
            errors.append(
                f"{relative_path}: {schema_path}.{keyword} requires explicit type {expected}"
            )
        elif not declared_types <= compatible_types:
            errors.append(
                f"{relative_path}: {schema_path}.{keyword} requires only type {expected}"
            )


def validate_schema_node(
    schema: dict[str, Any],
    relative_path: Path,
    schema_path: str = "$",
    *,
    runtime_profile: bool = False,
) -> list[str]:
    """Validate one schema node and every schema-position child recursively."""

    errors: list[str] = []
    for keyword in sorted(set(schema) - ADOPTED_SCHEMA_KEYWORDS):
        errors.append(
            f"{relative_path}: {schema_path}: unsupported JSON Schema keyword {keyword!r}"
        )

    declared_types = _validate_shapes(schema, relative_path, schema_path, errors)
    _validate_applicability(schema, declared_types, relative_path, schema_path, errors)
    if runtime_profile:
        _validate_runtime_profile(
            schema, declared_types, relative_path, schema_path, errors
        )

    for keyword in SCHEMA_MAP_KEYWORDS:
        if keyword not in schema:
            continue
        children = schema[keyword]
        keyword_path = f"{schema_path}.{keyword}"
        if not isinstance(children, dict):
            errors.append(f"{relative_path}: {keyword_path} must be an object")
            continue
        for name, child in children.items():
            child_path = f"{keyword_path}.{name}"
            if keyword == "patternProperties":
                _compile_pattern(name, relative_path, f"{keyword_path}[{name!r}]", errors)
            if not isinstance(child, dict):
                errors.append(f"{relative_path}: {child_path} must be an object")
                continue
            errors.extend(
                validate_schema_node(
                    child,
                    relative_path,
                    child_path,
                    runtime_profile=runtime_profile,
                )
            )

    if "items" in schema:
        child = schema["items"]
        child_path = f"{schema_path}.items"
        if not isinstance(child, dict):
            errors.append(f"{relative_path}: {child_path} must be an object")
        else:
            errors.extend(
                validate_schema_node(
                    child,
                    relative_path,
                    child_path,
                    runtime_profile=runtime_profile,
                )
            )

    for keyword in SCHEMA_LIST_KEYWORDS:
        if keyword not in schema:
            continue
        children = schema[keyword]
        keyword_path = f"{schema_path}.{keyword}"
        if not isinstance(children, list) or not children:
            errors.append(f"{relative_path}: {keyword_path} must be a non-empty array")
            continue
        for index, child in enumerate(children):
            child_path = f"{keyword_path}[{index}]"
            if not isinstance(child, dict):
                errors.append(f"{relative_path}: {child_path} must be an object")
                continue
            errors.extend(
                validate_schema_node(
                    child,
                    relative_path,
                    child_path,
                    runtime_profile=runtime_profile,
                )
            )

    for keyword in SCHEMA_NODE_KEYWORDS:
        if keyword not in schema:
            continue
        child = schema[keyword]
        child_path = f"{schema_path}.{keyword}"
        if not isinstance(child, dict):
            errors.append(f"{relative_path}: {child_path} must be an object")
            continue
        errors.extend(
            validate_schema_node(
                child,
                relative_path,
                child_path,
                runtime_profile=runtime_profile,
            )
        )

    for keyword in BOOLEAN_OR_SCHEMA_KEYWORDS:
        if keyword not in schema:
            continue
        child = schema[keyword]
        child_path = f"{schema_path}.{keyword}"
        if isinstance(child, dict):
            errors.extend(
                validate_schema_node(
                    child,
                    relative_path,
                    child_path,
                    runtime_profile=runtime_profile,
                )
            )
        elif not isinstance(child, bool):
            errors.append(f"{relative_path}: {child_path} must be a boolean or object")

    return errors
