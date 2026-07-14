#!/usr/bin/env python3
"""Validate a SpecRail workflow pack without network or GitHub writes."""

from __future__ import annotations

import argparse
import importlib.util
import json
import re
import sys
from pathlib import Path

from specrail_lib import (
    SpecRailError,
    load_pack,
    read_text,
    validate_action_policy,
    validate_labels,
    validate_state_graph,
    validate_skills_lock,
)
from sensitive_enforcement import validate_sensitive_registry


REQUIRED_FILES = [
    "AGENT_USAGE.md",
    "AGENTS.md",
    "workflow.yaml",
    "states.yaml",
    "labels.yaml",
    "checks/check_workflow.py",
    "checks/github_issue_evidence.py",
    "checks/github_pr_evidence.py",
    "checks/pack_asset_validation.py",
    "checks/pr_gate.py",
    "checks/route_gate.py",
    "checks/review_json_gate.py",
    "checks/specrail_lib.py",
    "tools/install_codex_skills.py",
    "skills-lock.json",
    "templates/issue_bug.md",
    "templates/issue_feature.md",
    "templates/product_spec.md",
    "templates/tech_spec.md",
    "templates/tasks.md",
    "templates/pull_request.md",
    "templates/tranche_checkpoint.md",
    "templates/zh-CN/issue_bug.md",
    "templates/zh-CN/issue_feature.md",
    "templates/zh-CN/product_spec.md",
    "templates/zh-CN/tech_spec.md",
    "templates/zh-CN/tasks.md",
    "templates/zh-CN/pull_request.md",
    "templates/zh-CN/tranche_checkpoint.md",
    "review/agent_first_review.md",
    "review/human_final_review.md",
    "policies/security_disclosure.md",
    "policies/maintainer_escalation.md",
    "schemas/flow_manifest.schema.json",
    "schemas/issue_triage.schema.json",
    "schemas/issue_evidence.schema.json",
    "schemas/evaluation_result.schema.json",
    "schemas/adoption_matrix.schema.json",
    "schemas/spec_packet.schema.json",
    "schemas/task_plan.schema.json",
    "schemas/pr_review_gate.schema.json",
    "schemas/review_result.schema.json",
    "schemas/workflow_run.schema.json",
]

REQUIRED_TOKENS = {
    "workflow.yaml": [
        "default_mode: dry_run",
        "forbidden_agent_actions:",
        "required_human_gates:",
        "action_policy:",
    ],
    "states.yaml": [
        "ready_to_spec",
        "ready_to_implement",
        "agent_review",
        "human_review",
        "merge_ready",
    ],
    "labels.yaml": [
        "readiness:",
        "ready_to_spec",
        "ready_to_implement",
        "security_private",
    ],
    "templates/product_spec.md": [
        "## Goals",
        "## Non-Goals",
        "## Acceptance Criteria",
    ],
    "templates/tech_spec.md": [
        "## Proposed Design",
        "## Test Plan",
        "## Rollback Plan",
    ],
    "templates/tasks.md": [
        "## Implementation Tasks",
        "## Verification",
        "## Handoff Notes",
    ],
    "templates/pull_request.md": [
        "## Linked Work",
        "## Readiness Gate",
        "## Review Gate",
        "## Merge Gate",
        "## Verification",
    ],
}


def validate_required_files(repo: Path) -> list[str]:
    errors: list[str] = []
    for rel in REQUIRED_FILES:
        path = repo / rel
        if not path.is_file():
            errors.append(f"missing required file: {rel}")
    return errors


def validate_tokens(repo: Path) -> list[str]:
    errors: list[str] = []
    for rel, tokens in REQUIRED_TOKENS.items():
        path = repo / rel
        if not path.is_file():
            continue
        text = read_text(path)
        for token in tokens:
            if token not in text:
                errors.append(f"{rel}: missing token {token!r}")
    return errors


def validate_pack_assets(repo: Path) -> list[str]:
    """Load the trusted helper for SpecRail-owned schemas and templates."""

    helper_path = Path(__file__).with_name("pack_asset_validation.py")
    if not helper_path.is_file():
        return [
            "cannot load trusted pack asset validation: "
            "checks/pack_asset_validation.py is missing"
        ]
    try:
        spec = importlib.util.spec_from_file_location(
            "_specrail_trusted_pack_asset_validation",
            helper_path,
        )
        if spec is None or spec.loader is None:
            return ["cannot load trusted pack asset validation: no module loader"]
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        validate_json_schemas = getattr(module, "validate_json_schemas", None)
        validate_template_parity = getattr(module, "validate_template_parity", None)
        missing_validators = [
            name
            for name, validator in (
                ("validate_json_schemas", validate_json_schemas),
                ("validate_template_parity", validate_template_parity),
            )
            if not callable(validator)
        ]
        if missing_validators:
            return [
                "trusted pack asset validation missing callable validator(s): "
                + ", ".join(missing_validators)
            ]
        return validate_json_schemas(repo) + validate_template_parity(repo)
    except Exception as exc:
        return [f"cannot run trusted pack asset validation: {exc}"]


def validate_all_json_schemas(repo: Path) -> list[str]:
    """Preserve remem's validation of schemas beyond the SpecRail-owned set."""

    errors: list[str] = []
    schema_dir = repo / "schemas"
    if not schema_dir.is_dir():
        return ["missing schemas/ directory"]
    for path in sorted(schema_dir.glob("*.schema.json")):
        relative_path = path.relative_to(repo)
        try:
            data = json.loads(read_text(path))
        except json.JSONDecodeError as exc:
            errors.append(f"{relative_path}: invalid JSON: {exc.msg}")
            continue
        if not isinstance(data, dict):
            errors.append(f"{relative_path}: top-level JSON must be an object")
            continue
        if "$schema" not in data:
            errors.append(f"{relative_path}: missing $schema")
        if "title" not in data:
            errors.append(f"{relative_path}: missing title")
        if data.get("type") != "object":
            errors.append(f"{relative_path}: top-level type must be object")
    return errors


def validate_template_parity(repo: Path) -> list[str]:
    """Validate the complete template surface adopted by remem."""

    errors: list[str] = []
    root = repo / "templates"
    zh = root / "zh-CN"
    base_files = sorted(path.name for path in root.glob("*.md"))
    zh_files = sorted(path.name for path in zh.glob("*.md")) if zh.is_dir() else []
    for name in base_files:
        if name not in zh_files:
            errors.append(f"templates/zh-CN: missing localized template {name}")
    for name in zh_files:
        if name not in base_files:
            errors.append(f"templates/zh-CN/{name}: no matching base template")
    stable_tokens = ["GH-", "ready_to_spec", "ready_to_implement"]
    for name in ["issue_feature.md", "product_spec.md", "tech_spec.md", "pull_request.md"]:
        for rel in [Path("templates") / name, Path("templates/zh-CN") / name]:
            path = repo / rel
            if not path.is_file():
                continue
            text = read_text(path)
            for token in stable_tokens:
                if token in read_text(repo / "templates" / name) and token not in text:
                    errors.append(f"{rel}: missing stable token {token}")
    return errors


def validate_spec_packet(spec_dir: Path) -> list[str]:
    errors: list[str] = []
    if not spec_dir.exists():
        return [f"spec packet does not exist: {spec_dir}"]
    if not spec_dir.is_dir():
        return [f"spec packet is not a directory: {spec_dir}"]

    match = re.fullmatch(r"GH([0-9]+)", spec_dir.name)
    if not match:
        errors.append(f"{spec_dir}: spec packet directory must be named GH<number>")
        issue_number = None
    else:
        issue_number = match.group(1)

    issue_tokens = []
    if issue_number:
        issue_tokens = [f"GH-{issue_number}", f"GH{issue_number}", f"#{issue_number}"]

    for name in ["product.md", "tech.md"]:
        path = spec_dir / name
        if not path.is_file():
            errors.append(f"{spec_dir}: missing {name}")
            continue
        text = read_text(path)
        if not text.strip():
            errors.append(f"{path}: must not be empty")
        if issue_tokens and not any(token in text for token in issue_tokens):
            errors.append(f"{path}: missing linked issue token {' or '.join(issue_tokens)}")

    task_path = spec_dir / "tasks.md"
    if not task_path.is_file():
        errors.append(f"{spec_dir}: missing tasks.md")
    else:
        errors.extend(validate_task_plan(task_path, issue_number))
    return errors


def spec_packet_sort_key(spec_dir: Path) -> tuple[int, int, str]:
    match = re.fullmatch(r"GH([0-9]+)", spec_dir.name)
    if match:
        return (0, int(match.group(1)), spec_dir.name)
    return (1, 0, str(spec_dir))


def discover_spec_packet_dirs(repo: Path) -> list[Path]:
    specs_dir = repo / "specs"
    if not specs_dir.is_dir():
        return []
    return sorted(
        [
            path.resolve()
            for path in specs_dir.iterdir()
            if path.is_dir() and re.fullmatch(r"GH([0-9]+)", path.name)
        ],
        key=spec_packet_sort_key,
    )


def select_spec_packet_dirs(
    repo: Path,
    raw_spec_dirs: list[str],
    *,
    all_specs: bool,
) -> list[Path]:
    spec_dirs: list[Path] = []
    if all_specs:
        spec_dirs.extend(discover_spec_packet_dirs(repo))
    spec_dirs.extend((repo / raw_spec_dir).resolve() for raw_spec_dir in raw_spec_dirs)

    unique_spec_dirs: list[Path] = []
    seen: set[Path] = set()
    for spec_dir in spec_dirs:
        if spec_dir in seen:
            continue
        seen.add(spec_dir)
        unique_spec_dirs.append(spec_dir)

    if all_specs:
        return sorted(unique_spec_dirs, key=spec_packet_sort_key)
    return unique_spec_dirs


def validate_task_plan(path: Path, issue_number: str | None) -> list[str]:
    errors: list[str] = []
    text = read_text(path)
    if not text.strip():
        return [f"{path}: must not be empty"]
    prefix = f"SP{issue_number}-T" if issue_number else "SP"
    ids: list[str] = []
    for line_number, line in enumerate(text.splitlines(), start=1):
        if "- [" not in line:
            continue
        match = re.search(r"`([^`]+)`", line)
        if not match:
            errors.append(f"{path}:{line_number}: task is missing stable ID")
            continue
        task_id = match.group(1)
        ids.append(task_id)
        if issue_number and not task_id.startswith(prefix):
            errors.append(f"{path}:{line_number}: task ID {task_id} must start with {prefix}")
        for token in ["Owner:", "Done when:", "Verify:"]:
            if token not in line:
                errors.append(f"{path}:{line_number}: task {task_id} missing {token}")
    if not ids:
        errors.append(f"{path}: no task checklist items found")
    duplicates = sorted({task_id for task_id in ids if ids.count(task_id) > 1})
    for duplicate in duplicates:
        errors.append(f"{path}: duplicate task ID {duplicate}")
    return errors


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Validate a SpecRail workflow pack."
    )
    parser.add_argument("--repo", default=".", help="Workflow pack root")
    parser.add_argument(
        "--spec-dir",
        action="append",
        default=[],
        help="Optional specs/GH<number> directory to validate",
    )
    parser.add_argument(
        "--all-specs",
        action="store_true",
        help="Validate every specs/GH<number> directory under the repo",
    )
    args = parser.parse_args()

    repo = Path(args.repo).resolve()
    errors: list[str] = []
    try:
        config = load_pack(repo)
        errors.extend(validate_required_files(repo))
        errors.extend(validate_tokens(repo))
        errors.extend(validate_pack_assets(repo))
        errors.extend(validate_all_json_schemas(repo))
        errors.extend(validate_state_graph(config))
        errors.extend(validate_labels(config))
        errors.extend(validate_action_policy(config))
        errors.extend(validate_sensitive_registry(config))
        errors.extend(validate_skills_lock(repo))
        errors.extend(validate_template_parity(repo))
        for spec_dir in select_spec_packet_dirs(
            repo,
            args.spec_dir,
            all_specs=args.all_specs,
        ):
            errors.extend(validate_spec_packet(spec_dir))
    except SpecRailError as exc:
        errors.append(str(exc))

    if errors:
        print("SpecRail check failed")
        for error in errors:
            print(f"- {error}")
        return 1

    print("SpecRail check passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
