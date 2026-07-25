#!/usr/bin/env python3
"""Evaluate whether a SpecRail action may proceed from local evidence."""

from __future__ import annotations

import argparse
import json
import shlex
import sys
from pathlib import Path, PurePosixPath
from typing import Any

from specrail_lib import (
    TERMINAL_BLOCKING_STATES,
    SpecRailError,
    action_policy,
    infer_state,
    load_pack,
    render_artifact_path,
    resolve_path,
    resolve_repo_path,
    resolve_spec_packet_root,
    spec_is_legacy,
    spec_packet_artifact_paths,
    state_map,
    validated_repo_relative_path,
    validate_action_policy,
    validate_labels,
    validate_state_graph,
)
from duplicate_work_gate import evaluate_duplicate_work_gate_path
from rejection_items import (
    add_prior_rejection_argument,
    apply_prior_rejection,
    finalize_items,
    item_from_missing,
    item_from_reason,
    make_item,
)
from sensitive_enforcement import (
    classification_from_approved_tech,
    evaluate_sensitive_evidence,
    sensitive_registry,
    trusted_default_base,
    validate_sensitive_registry,
)


ROUTE_ALIASES = {
    "action": "triage_issue",
    "triage": "triage_issue",
    "spec": "write_spec",
    "write-spec": "write_spec",
    "write_spec": "write_spec",
    "implement": "implement",
    "review": "review_pr",
    "review-pr": "review_pr",
    "review_pr": "review_pr",
    "fix-ci": "fix_ci",
    "fix_ci": "fix_ci",
    "release-note": "draft_release_note",
    "draft-release-note": "draft_release_note",
    "draft_release_note": "draft_release_note",
}

ARTIFACT_FILES = {
    "product_spec",
    "tech_spec",
    "task_plan",
}
READINESS_GATED_ROUTES = {"write_spec", "implement"}
DECISION_RANK = {
    "allowed": 0,
    "warn": 1,
    "needs_human": 2,
    "blocked": 3,
}


def normalize_route(raw: str) -> str:
    route = ROUTE_ALIASES.get(raw, raw)
    return route.replace("-", "_")


def parse_artifact_value(raw: str) -> tuple[str, str]:
    name, sep, value = raw.partition("=")
    if not sep or not name.strip() or not value.strip():
        raise SpecRailError(f"invalid --artifact {raw!r}; expected name=value")
    return name.strip(), value.strip()


def load_evidence(path: Path | None) -> dict[str, Any]:
    if path is None:
        return {}
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
    except OSError as exc:
        raise SpecRailError(f"cannot read evidence file {path}: {exc}") from exc
    except json.JSONDecodeError as exc:
        raise SpecRailError(f"invalid evidence JSON {path}: {exc.msg}") from exc
    if not isinstance(data, dict):
        raise SpecRailError("evidence JSON must be an object")
    return data


def artifact_exists(repo: Path, artifact_path: str | None) -> bool:
    if not artifact_path:
        return False
    return resolve_repo_path(
        repo,
        artifact_path,
        label="artifact path",
    ).is_file()


def stricter_decision(current: str, candidate: str) -> str:
    if DECISION_RANK[candidate] > DECISION_RANK[current]:
        return candidate
    return current


def required_artifact_path(config: Any, artifact: str, issue: int | None) -> str | None:
    if artifact == "linked_issue":
        return None
    if artifact == "linked_pr":
        return None
    if artifact == "verification":
        return None
    if artifact in ARTIFACT_FILES and issue is not None:
        return spec_packet_artifact_paths(config, issue)[artifact]
    return render_artifact_path(config, artifact, issue)


def evaluate_route(args: argparse.Namespace) -> dict[str, Any]:
    repo = resolve_path(Path(args.repo), label="repository")
    config = load_pack(repo)
    config_errors: list[str] = []
    config_errors.extend(validate_state_graph(config))
    config_errors.extend(validate_labels(config))
    config_errors.extend(validate_action_policy(config))
    config_errors.extend(validate_sensitive_registry(config))
    try:
        configured_spec_paths = spec_packet_artifact_paths(config, 1)
        configured_spec_root = PurePosixPath(
            configured_spec_paths["spec_packet"]
        ).parent
        resolve_spec_packet_root(repo, configured_spec_root)
    except SpecRailError as exc:
        config_errors.append(str(exc))

    route = normalize_route(args.route)
    policies = action_policy(config)
    policy = policies.get(route)
    if policy is None:
        config_errors.append(f"unknown route: {route}")

    evidence = load_evidence(Path(args.evidence) if args.evidence else None)
    labels = list(args.label or [])
    labels.extend(str(label) for label in evidence.get("labels", []) if str(label).strip())
    evidence_state = evidence.get("state")
    explicit_state = args.state or evidence_state
    github_state = str(evidence.get("github_state") or "").upper()
    state_from_cli = args.state is not None
    state_from_evidence = not state_from_cli and evidence_state is not None
    state_source = str(evidence.get("state_source") or "none")
    state_trusted = state_source == "label" and evidence.get("state_trusted") is True

    reasons: list[str] = []
    satisfied: list[str] = []
    missing: list[str] = []
    items: list[dict[str, str]] = []
    blocked_actions: list[str] = []
    allowed_actions: list[str] = []
    required_artifacts: list[str] = []
    human_gates: list[str] = []
    duplicate_work_result: dict[str, Any] | None = None
    sensitive_classification: dict[str, Any] | None = None
    sensitive_errors: list[str] = []

    if config_errors:
        return {
            "decision": "blocked",
            "route": route,
            "current_state": explicit_state,
            "issue": args.issue,
            "pr": args.pr,
            "reasons": config_errors,
            "satisfied": [],
            "missing": [],
            "rejection_items": finalize_items(
                item_from_reason(error, "config_error") for error in config_errors
            ),
            "required_artifacts": [],
            "human_gates": [],
            "allowed_actions": [],
            "blocked_actions": [route],
            "verification_commands": ["python3 checks/check_workflow.py --repo ."],
        }

    if github_state and github_state != "OPEN":
        return blocked_result(
            route,
            explicit_state,
            args,
            [f"GitHub issue state must be OPEN; got {github_state}"],
        )

    current_state, state_evidence = infer_state(config, explicit_state, labels)
    if state_from_evidence and current_state == evidence_state:
        state_evidence = [f"state provided by evidence: {current_state} ({state_source})"]
    satisfied.extend(state_evidence)

    states = state_map(config)
    if current_state and current_state not in states:
        return blocked_result(
            route,
            current_state,
            args,
            [f"unknown current state: {current_state}"],
        )

    if current_state in TERMINAL_BLOCKING_STATES:
        return blocked_result(
            route,
            current_state,
            args,
            [f"state {current_state} is terminal or maintainer-reserved"],
        )

    assert policy is not None
    allowed_from = [str(state) for state in policy.get("allowed_from", [])]
    required = [str(artifact) for artifact in policy.get("required_artifacts", [])]
    creates = [str(artifact) for artifact in policy.get("creates_artifacts", [])]
    human_gates = [str(gate) for gate in policy.get("human_gates", [])]

    if current_state is None:
        missing.append("current_state")
        reasons.append("no state or matching readiness label was provided")
        items.append(
            make_item(
                "missing_evidence_field",
                "current_state",
                "current_state provided via --state or a readiness label",
                "absent",
            )
        )
    elif current_state in allowed_from:
        satisfied.append(f"state {current_state} allows {route}")
    else:
        missing.append(f"allowed_state:{'|'.join(allowed_from)}")
        reasons.append(
            f"route {route} requires one of {', '.join(allowed_from)}; got {current_state}"
        )
        items.append(
            make_item(
                "invalid_state",
                f"allowed_state:{route}",
                f"state in {', '.join(allowed_from)}",
                str(current_state),
            )
        )

    if (
        route in READINESS_GATED_ROUTES
        and "readiness_label" in human_gates
        and state_from_evidence
        and current_state in allowed_from
        and not state_trusted
    ):
        missing.append("trusted_state")
        reasons.append(
            f"state {current_state} came from untrusted {state_source} evidence; "
            "maintainer readiness label required"
        )
        items.append(
            make_item(
                "invalid_state",
                "trusted_state",
                "state evidence trusted via maintainer readiness label",
                f"state {current_state} from untrusted {state_source} evidence",
            )
        )

    provided_artifacts = dict(evidence.get("artifacts", {})) if isinstance(evidence.get("artifacts"), dict) else {}
    for raw_artifact in args.artifact or []:
        name, value = parse_artifact_value(raw_artifact)
        provided_artifacts[name] = value

    for artifact in required:
        if artifact == "linked_issue":
            if args.issue is None:
                missing.append("linked_issue")
                items.append(item_from_missing("linked_issue"))
            else:
                satisfied.append(f"linked_issue: GH-{args.issue}")
            continue
        if artifact == "linked_pr":
            if args.pr is None:
                missing.append("linked_pr")
                items.append(item_from_missing("linked_pr"))
            else:
                satisfied.append(f"linked_pr: PR-{args.pr}")
            continue
        if artifact == "verification":
            verification = evidence.get("verification") or provided_artifacts.get("verification")
            if verification:
                satisfied.append("verification evidence provided")
            else:
                missing.append("verification")
                items.append(item_from_missing("verification"))
            continue
        provided = provided_artifacts.get(artifact)
        if artifact in ARTIFACT_FILES and args.issue is None:
            required_artifacts.append(str(provided) if provided else artifact)
            if provided and artifact_exists(repo, str(provided)):
                satisfied.append(f"{artifact}: {provided}")
            elif provided:
                missing.append(f"{artifact}:{provided}")
                items.append(
                    make_item(
                        "missing_artifact",
                        artifact,
                        f"{artifact} exists at {provided}",
                        "absent",
                    )
                )
            else:
                missing.append(artifact)
                items.append(
                    make_item(
                        "missing_artifact",
                        artifact,
                        f"{artifact} path provided in evidence",
                        "absent",
                    )
                )
            continue
        path = required_artifact_path(config, artifact, args.issue)
        required_artifacts.append(path or artifact)
        if provided:
            if artifact in ARTIFACT_FILES:
                try:
                    normalized_provided = validated_repo_relative_path(
                        str(provided),
                        label=f"{artifact} evidence path",
                    ).as_posix()
                except SpecRailError as exc:
                    missing.append(f"{artifact}:{path}")
                    reasons.append(str(exc))
                    items.append(
                        make_item(
                            "invalid_evidence_value",
                            artifact,
                            f"{artifact} at configured path {path}",
                            str(exc),
                        )
                    )
                    continue
                if normalized_provided != path:
                    missing.append(f"{artifact}:{path}")
                    reasons.append(
                        f"{artifact} provided at {provided} does not match "
                        f"configured path {path}"
                    )
                    items.append(
                        make_item(
                            "invalid_evidence_value",
                            artifact,
                            f"{artifact} at configured path {path}",
                            f"provided at {provided}",
                        )
                    )
                elif not artifact_exists(repo, normalized_provided):
                    missing.append(f"{artifact}:{normalized_provided}")
                    items.append(
                        make_item(
                            "missing_artifact",
                            artifact,
                            f"{artifact} exists at {normalized_provided}",
                            "absent",
                        )
                    )
                else:
                    satisfied.append(f"{artifact}: {normalized_provided}")
            else:
                satisfied.append(f"{artifact}: {provided}")
            continue
        if artifact in ARTIFACT_FILES:
            if artifact_exists(repo, path):
                satisfied.append(f"{artifact}: {path}")
            else:
                missing.append(f"{artifact}:{path}")
                items.append(
                    make_item(
                        "missing_artifact",
                        artifact,
                        f"{artifact} exists at {path}",
                        "absent",
                    )
                )
        elif path:
            required_artifacts.append(path)

    legacy_spec = False
    if route == "implement":
        # GH142 B-005/B-007: a legacy spec packet is never a basis for
        # implementation; an unreadable product.md fails closed via the
        # SpecRailError path in main().
        if args.issue is not None and spec_is_legacy(repo, config, args.issue):
            legacy_spec = True
            legacy_packet = spec_packet_artifact_paths(config, args.issue)[
                "spec_packet"
            ]
            missing.append("non_legacy_spec")
            reasons.append(
                f"spec packet {legacy_packet} is status: legacy; "
                "rewrite via write_spec (needs_spec) before implementing"
            )
            items.append(
                make_item(
                    "contract_violation",
                    "non_legacy_spec",
                    f"spec packet {legacy_packet} without a status: legacy "
                    "declaration in its Linked Issue section",
                    "product.md declares status: legacy in its Linked Issue "
                    "section",
                )
            )
        trusted_classification: dict[str, Any] | None = None
        sensitive_input = dict(evidence)
        sensitive_input.pop("sensitive_classification", None)
        registry = sensitive_registry(config)
        if registry["paths"] or registry["specs"]:
            try:
                _trusted_base_ref, trusted_base_sha = trusted_default_base(
                    repo,
                    default_base_ref=evidence.get("default_base_ref"),
                    default_base_sha=evidence.get("default_base_sha"),
                )
                trusted_classification = classification_from_approved_tech(
                    config,
                    repo,
                    issue=args.issue,
                    base_sha=trusted_base_sha,
                )
                sensitive_input["sensitive_classification"] = {
                    key: trusted_classification[key]
                    for key in [
                        "source", "changed_paths", "spec_refs", "matched_paths",
                        "matched_specs", "registry_configured",
                        "enforcement_sensitive",
                    ]
                }
            except (SpecRailError, TypeError) as exc:
                sensitive_errors.append(str(exc))
        sensitive_classification, sensitive_satisfied, evaluated_sensitive_errors = (
            evaluate_sensitive_evidence(
                config,
                repo,
                sensitive_input,
                expected_source="tech_spec",
                issue=args.issue,
                expected_base_ref=evidence.get("base_ref"),
                expected_base_head=evidence.get("base_sha"),
            )
        )
        sensitive_errors.extend(evaluated_sensitive_errors)
        if trusted_classification is not None:
            sensitive_classification = trusted_classification
        satisfied.extend(sensitive_satisfied)
        if sensitive_errors:
            reasons.extend(sensitive_errors)
            missing.append("sensitive_enforcement")
            items.append(item_from_missing("sensitive_enforcement"))
            items.extend(
                item_from_reason(error, "contract_violation")
                for error in sensitive_errors
            )
        if args.issue is None:
            duplicate_work_result = {
                "decision": "needs_human",
                "issue": None,
                "reasons": [
                    "duplicate work evidence cannot be evaluated until a linked issue is provided"
                ],
                "satisfied": [],
                "missing": ["duplicate_evidence"],
                "blocked_actions": ["implement"],
                "verification_commands": [
                    "python3 checks/github_duplicate_evidence.py "
                    "--github-repo OWNER/REPO --issue <issue> --json"
                ],
            }
        else:
            duplicate_work_result = evaluate_duplicate_work_gate_path(
                repo,
                args.issue,
                Path(args.duplicate_evidence) if args.duplicate_evidence else None,
            )
        for item in duplicate_work_result.get("satisfied", []):
            satisfied.append(f"duplicate_work: {item}")
        for item in duplicate_work_result.get("missing", []):
            missing.append(f"duplicate_work:{item}")
            items.append(item_from_missing(f"duplicate_work:{item}"))
        duplicate_work_allowed = duplicate_work_result.get("decision") == "allowed"
        for reason in duplicate_work_result.get("reasons", []):
            reasons.append(f"duplicate_work: {reason}")
            if not duplicate_work_allowed:
                items.append(
                    item_from_reason(f"duplicate_work: {reason}", "contract_violation")
                )

    for action, action_body in policies.items():
        allowed_states = [str(state) for state in action_body.get("allowed_from", [])]
        if current_state and current_state in allowed_states:
            allowed_actions.append(action)

    if route in {"review_pr", "draft_release_note"}:
        blocked_actions.extend(["final_approval", "merge"])
    else:
        blocked_actions.extend(["final_approval", "merge", "force_push"])

    if missing:
        if (
            current_state is None
            or any(item.startswith("allowed_state:") for item in missing)
            or "trusted_state" in missing
        ):
            decision = "needs_human" if human_gates else "blocked"
        else:
            decision = "warn" if args.mode in {"dry_run", "advisory"} else "blocked"
    else:
        decision = "allowed"
        reasons.append(f"route {route} passed local SpecRail gates")

    if duplicate_work_result is not None:
        decision = stricter_decision(decision, str(duplicate_work_result["decision"]))
    if sensitive_errors:
        decision = "blocked"
    if legacy_spec:
        # GH142 B-005: legacy blocks in every mode; dry_run must not soften
        # this to warn.
        decision = "blocked"

    for artifact in creates:
        if args.issue is None:
            required_artifacts.append(artifact)
            continue
        path = render_artifact_path(config, artifact, args.issue)
        if path:
            required_artifacts.append(path)

    verification_commands = ["python3 checks/check_workflow.py --repo ."]
    if args.issue:
        spec_dir = spec_packet_artifact_paths(config, args.issue, repo=repo)["spec_packet"]
        verification_commands.append(
            "python3 checks/check_workflow.py --repo . --spec-dir="
            + shlex.quote(spec_dir)
        )

    return {
        "decision": decision,
        "route": route,
        "mode": args.mode,
        "current_state": current_state,
        "issue": args.issue,
        "pr": args.pr,
        "reasons": reasons,
        "satisfied": sorted(set(satisfied)),
        "missing": sorted(set(missing)),
        "rejection_items": [] if decision == "allowed" else finalize_items(items),
        "required_artifacts": sorted(set(required_artifacts)),
        "human_gates": human_gates,
        "allowed_actions": sorted(set(allowed_actions)),
        "blocked_actions": sorted(set(blocked_actions)),
        "duplicate_work_gate": duplicate_work_result,
        "sensitive_classification": sensitive_classification,
        "verification_commands": verification_commands,
    }


def blocked_result(
    route: str,
    current_state: str | None,
    args: argparse.Namespace,
    reasons: list[str],
    item_category: str = "invalid_state",
) -> dict[str, Any]:
    return {
        "decision": "blocked",
        "route": route,
        "mode": args.mode,
        "current_state": current_state,
        "issue": args.issue,
        "pr": args.pr,
        "reasons": reasons,
        "satisfied": [],
        "missing": [],
        "rejection_items": finalize_items(
            item_from_reason(reason, item_category) for reason in reasons
        ),
        "required_artifacts": [],
        "human_gates": [],
        "allowed_actions": [],
        "blocked_actions": [route],
        "verification_commands": ["python3 checks/check_workflow.py --repo ."],
    }


def print_human(result: dict[str, Any]) -> None:
    print(f"decision: {result['decision']}")
    print(f"route: {result['route']}")
    if result.get("current_state"):
        print(f"current_state: {result['current_state']}")
    if result.get("issue"):
        print(f"issue: GH-{result['issue']}")
    if result.get("pr"):
        print(f"pr: PR-{result['pr']}")
    if result.get("reasons"):
        print("reasons:")
        for reason in result["reasons"]:
            print(f"- {reason}")
    if result.get("missing"):
        print("missing:")
        for item in result["missing"]:
            print(f"- {item}")
    if result.get("required_artifacts"):
        print("required_artifacts:")
        for item in result["required_artifacts"]:
            print(f"- {item}")
    if result.get("verification_commands"):
        print("verification_commands:")
        for command in result["verification_commands"]:
            print(f"- {command}")


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Evaluate a SpecRail route from local evidence."
    )
    parser.add_argument("--repo", default=".", help="SpecRail pack or adopted repo root")
    parser.add_argument("--route", "--action", required=True, help="Route/action to evaluate")
    parser.add_argument("--issue", type=int, help="Linked GitHub issue number")
    parser.add_argument("--pr", type=int, help="Linked pull request number")
    parser.add_argument("--state", help="Canonical SpecRail state")
    parser.add_argument("--label", action="append", default=[], help="Issue/PR label evidence")
    parser.add_argument(
        "--artifact",
        action="append",
        default=[],
        help="Artifact evidence in name=path form",
    )
    parser.add_argument("--evidence", help="Optional JSON evidence file")
    parser.add_argument("--duplicate-evidence", help="Optional duplicate work evidence JSON file")
    parser.add_argument(
        "--mode",
        default="dry_run",
        choices=["dry_run", "advisory", "required"],
        help="Evaluation enforcement mode",
    )
    parser.add_argument("--json", action="store_true", help="Print JSON output")
    add_prior_rejection_argument(parser)
    args = parser.parse_args()

    try:
        result = evaluate_route(args)
    except SpecRailError as exc:
        result = {
            "decision": "blocked",
            "route": normalize_route(args.route),
            "mode": args.mode,
            "current_state": args.state,
            "issue": args.issue,
            "pr": args.pr,
            "reasons": [str(exc)],
            "satisfied": [],
            "missing": [],
            "rejection_items": finalize_items(
                [item_from_reason(str(exc), "config_error")]
            ),
            "required_artifacts": [],
            "human_gates": [],
            "allowed_actions": [],
            "blocked_actions": [normalize_route(args.route)],
            "verification_commands": ["python3 checks/check_workflow.py --repo ."],
        }

    result = apply_prior_rejection(
        result, args.prior_rejection, blocked_actions=[result["route"]]
    )

    if args.json:
        print(json.dumps(result, indent=2, sort_keys=True))
    else:
        print_human(result)

    if result["decision"] == "blocked":
        return 1
    if result["decision"] == "needs_human" and args.mode == "required":
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
