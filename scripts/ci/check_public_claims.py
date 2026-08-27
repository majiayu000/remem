#!/usr/bin/env python3
"""Guard README/release surfaces against unsupported public benchmark claims."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import subprocess
import sys
from pathlib import Path
from pathlib import PurePosixPath


ROOT = Path(__file__).resolve().parents[2]
BASELINE_REPORT = ROOT / "eval/public/reports/baseline.json"
CLAIM_REGISTRY = ROOT / "eval/claims/registry.json"
CLAIM_CONTRACT = ROOT / "eval/public/claims/coding-claim-contract-v1.json"

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

NEGATED_STRONG_CLAIM_RE = re.compile(
    r"\b(do not|don't|does not|must not|cannot|unsupported|not support(?:ed)?|"
    r"no public)\b[^.\n]{0,120}\b(SOTA|state[- ]of[- ]the[- ]art|best|beats?|"
    r"outperforms?|superior(?:ity)?|coding[- ]task superiority|"
    r"coding[- ]agent outcome improvement)\b",
    re.I,
)

REPORT_LINK_RE = re.compile(
    r"(eval/public/reports/baseline\.(?:json|md)|public-baseline-directional-v1)",
    re.I,
)

CLAIM_MARKER_RE = re.compile(r"<!--\s*remem-claim:([a-z0-9][a-z0-9-]*)\s*-->")

RELEASE_POLICY_CONTRACT_LINES = {
    "| 2 | Coding-agent outcome improvement | A passing artifact verifier; the #931 `no_memory` / `remem_e2e` / `curated_file_budgeted` matrix on the registered 16-task set, exactly registered run indices 0/1/2 per task and condition; positive remem delta versus `no_memory`; reported token/turn/wall-time regressions; and the coding outcome stop-loss gate. |",
    "| 3 | Public SOTA claim | A public benchmark comparison using the same model, budget, harness, and published artifacts; wording must name the benchmark and condition instead of generalizing to all long-term memory or all coding agents. |",
    "roadmap wording that says remem improves coding-agent outcomes, beats a",
    "maintained context file, or is broadly superior for coding workflows. The gate",
    "- `remem_e2e` beats `no_memory` on resolved rate by at least 10 percentage",
    "If `curated_file_budgeted` ties or beats remem with lower cost and no material usability",
}


def die(message: str) -> None:
    print(f"FAIL: {message}", file=sys.stderr)
    sys.exit(1)


def load_claim_gate() -> dict[str, object]:
    if not BASELINE_REPORT.is_file():
        die(f"missing baseline report: {BASELINE_REPORT.relative_to(ROOT)}")
    with BASELINE_REPORT.open("r", encoding="utf-8") as handle:
        report = json.load(handle)
    gate = report.get("claim_gate")
    if not isinstance(gate, dict):
        die("baseline report is missing claim_gate")
    registered_claims = load_registered_coding_claims()
    return {
        **gate,
        "registered_coding_claims_passed": registered_claims is not None,
        "registered_coding_claims": registered_claims or [],
    }


def load_registered_coding_claims() -> list[dict[str, object]] | None:
    if not CLAIM_REGISTRY.is_file() or not CLAIM_CONTRACT.is_file():
        return None
    try:
        registry = json.loads(CLAIM_REGISTRY.read_text(encoding="utf-8"))
        contract = json.loads(CLAIM_CONTRACT.read_text(encoding="utf-8"))
    except (OSError, subprocess.CalledProcessError, json.JSONDecodeError):
        return None
    if registry.get("schema_version") != 1 or registry.get("locked") is not True:
        return None
    if (
        contract.get("schema_version") != 1
        or contract.get("closed") is not True
        or contract.get("contract_id") != "gh931-coding-claims-v1"
    ):
        return None
    claims = registry.get("claims")
    contracts = contract.get("claims")
    if not isinstance(claims, list) or not isinstance(contracts, list):
        return None
    claim_ids = [item.get("id") for item in claims if isinstance(item, dict)]
    contract_ids = [item.get("id") for item in contracts if isinstance(item, dict)]
    if len(claim_ids) != len(claims) or sorted(claim_ids) != sorted(contract_ids):
        return None
    verified_claims: list[dict[str, object]] = []
    for expected_claim in contracts:
        if not isinstance(expected_claim, dict):
            return None
        claim_id = expected_claim.get("id")
        if not isinstance(claim_id, str):
            return None
        claim = next(
            (
                candidate
                for candidate in claims
                if isinstance(candidate, dict) and candidate.get("id") == claim_id
            ),
            None,
        )
        if not isinstance(claim, dict) or claim.get("status") != "PASS":
            return None
        if claim.get("comparison") != expected_claim.get("comparison"):
            return None
        if claim.get("metric") != expected_claim.get("metric"):
            return None
        allowed_wording = claim.get("allowed_wording")
        forbidden_wording = claim.get("forbidden_wording")
        if (
            allowed_wording != expected_claim.get("allowed_wording")
            or forbidden_wording != expected_claim.get("forbidden_wording")
        ):
            return None
        supporting_report = claim.get("supporting_report")
        if not isinstance(supporting_report, dict):
            return None
        relative = supporting_report.get("path")
        expected_hash = supporting_report.get("sha256")
        if not isinstance(relative, str) or not isinstance(expected_hash, str):
            return None
        if re.fullmatch(r"[0-9a-f]{64}", expected_hash) is None:
            return None
        path = PurePosixPath(relative)
        if path.is_absolute() or ".." in path.parts or not relative.startswith("eval/public/"):
            return None
        report_path = ROOT.joinpath(*path.parts)
        try:
            report_bytes = report_path.read_bytes()
            supporting = json.loads(report_bytes)
        except (OSError, json.JSONDecodeError):
            return None
        if hashlib.sha256(report_bytes).hexdigest() != expected_hash:
            return None
        if not supporting_report_matches_current_source(supporting):
            return None
        verified_claims.append(
            {
                "id": claim_id,
                "comparison": expected_claim["comparison"],
                "metric": expected_claim["metric"],
                "allowed_wording": allowed_wording,
                "forbidden_wording": forbidden_wording,
                "supporting_report": {
                    "path": relative,
                    "sha256": expected_hash,
                },
            }
        )
    return verified_claims


def production_input_tree_sha256() -> str | None:
    paths = [
        "Cargo.toml", "Cargo.lock", "build.rs", ".cargo", "rust-toolchain.toml",
        "src", "prompts", "assets", ":(exclude)src/eval/ship_matrix.rs",
        ":(exclude)src/eval/ship_matrix/**", ":(exclude)src/eval/gates.rs",
    ]
    try:
        output = subprocess.run(
            ["git", "ls-files", "-s", "--", *paths], cwd=ROOT, check=True,
            capture_output=True,
        ).stdout
        clean = all(
            subprocess.run(["git", *args, "--", *paths], cwd=ROOT).returncode == 0
            for args in (["diff", "--quiet"], ["diff", "--cached", "--quiet"])
        )
    except (OSError, subprocess.CalledProcessError):
        return None
    return hashlib.sha256(output).hexdigest() if output and clean else None


def supporting_report_matches_current_source(report: object) -> bool:
    if not isinstance(report, dict):
        return False
    reproducibility = report.get("reproducibility")
    if not isinstance(reproducibility, dict):
        return False
    commits = reproducibility.get("remem_commits")
    if not isinstance(commits, list) or not commits:
        return False
    if not all(isinstance(item, str) and re.fullmatch(r"[0-9a-f]{40}", item) for item in commits):
        return False
    expected_tree = reproducibility.get("production_input_tree_sha256")
    return isinstance(expected_tree, str) and expected_tree == production_input_tree_sha256()


def coding_claim_ready(gate: dict[str, object]) -> bool:
    return (
        gate.get("artifact_verifier_passed") is True
        and gate.get("registered_coding_claims_passed") is True
    )


def sota_claim_ready(gate: dict[str, object]) -> bool:
    return gate.get("public_sota_status") == "passed_level3_public_sota"


def line_is_policy_or_negative(text: str) -> bool:
    return NEGATED_STRONG_CLAIM_RE.search(text) is not None


def line_is_closed_policy_contract(text: str) -> bool:
    return text.strip() in RELEASE_POLICY_CONTRACT_LINES


def line_has_report_link(text: str) -> bool:
    return REPORT_LINK_RE.search(text) is not None


def registered_coding_claim_violation(
    text: str, gate: dict[str, object]
) -> str | None:
    markers = CLAIM_MARKER_RE.findall(text)
    if len(markers) != 1:
        return "coding claim must carry exactly one registered claim marker"
    claims = gate.get("registered_coding_claims")
    if not isinstance(claims, list):
        return "registered coding claim contract is unavailable"
    claim = next(
        (
            candidate
            for candidate in claims
            if isinstance(candidate, dict) and candidate.get("id") == markers[0]
        ),
        None,
    )
    if not isinstance(claim, dict):
        return "coding claim marker is not backed by a verified registry entry"
    forbidden = claim.get("forbidden_wording")
    if not isinstance(forbidden, list):
        return "registered coding claim has no forbidden-wording contract"
    lowered = text.casefold()
    if any(isinstance(phrase, str) and phrase.casefold() in lowered for phrase in forbidden):
        return "coding claim contains registry-forbidden wording"
    report = claim.get("supporting_report")
    allowed = claim.get("allowed_wording")
    if not isinstance(report, dict) or not isinstance(allowed, list):
        return "registered coding claim authority is incomplete"
    report_path = report.get("path")
    if not isinstance(report_path, str):
        return "registered coding claim has no exact supporting-report path"
    expected_lines = {
        f"<!-- remem-claim:{markers[0]} --> {wording} [evidence]({report_path})"
        for wording in allowed
        if isinstance(wording, str)
    }
    if text.strip() not in expected_lines:
        return (
            "coding claim must exactly match registry allowed_wording and its "
            "hash-bound supporting-report path"
        )
    return None


def classify_violation(
    text: str, gate: dict[str, object], _context: str | None = None
) -> str | None:
    if not STRONG_CLAIM_RE.search(text):
        return None
    # Authorization is line-local. Adjacent headings and prose cannot turn an
    # otherwise unsupported claim into policy or negative wording.
    if line_is_policy_or_negative(text) or line_is_closed_policy_contract(text):
        return None

    if SOTA_CLAIM_RE.search(text):
        if sota_claim_ready(gate) and line_has_report_link(text):
            return None
        return "SOTA/best claim lacks a passed Level 3 public claim gate and report link"

    if CODING_CLAIM_RE.search(text):
        if not coding_claim_ready(gate):
            return (
                "coding-outcome superiority claim lacks verified registry authority "
                "and a hash-bound supporting report"
            )
        return registered_coding_claim_violation(text, gate)

    return "strong public claim is not grounded in an approved report"


def check_surfaces(gate: dict[str, object]) -> list[str]:
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
    registered_claim = {
        "id": "remem-e2e-vs-no-memory-v1",
        "comparison": {"treatment": "remem_e2e", "control": "no_memory"},
        "metric": "resolved_rate",
        "allowed_wording": [
            "On the registered fixture, remem_e2e outperforms no_memory on resolved_rate."
        ],
        "forbidden_wording": ["universally", "every coding workload"],
        "supporting_report": {
            "path": "eval/public/reports/coding-claim.json",
            "sha256": "1" * 64,
        },
    }
    blocked_gate = {
        "artifact_verifier_passed": True,
        "coding_claim_level": "directional_only_no_public_claim",
        "coding_outcome_stop_loss_status": "not_evaluated_insufficient_coding_matrix",
        "public_sota_status": "not_evaluated_no_public_sota_claim",
    }
    evaluation_ready_gate = {
        "artifact_verifier_passed": True,
        "coding_claim_level": "directional_only_no_public_claim",
        "coding_outcome_stop_loss_status": "ready_for_stop_loss_evaluation",
        "public_sota_status": "not_evaluated_no_public_sota_claim",
    }
    sota_evaluation_ready_gate = {
        **evaluation_ready_gate,
        "public_sota_status": "ready_for_level3_evaluation",
    }
    sota_unknown_gate = {
        **evaluation_ready_gate,
        "public_sota_status": "unknown",
    }
    passed_gate = {
        "artifact_verifier_passed": True,
        "coding_claim_level": "directional_only_no_public_claim",
        "coding_outcome_stop_loss_status": "ready_for_stop_loss_evaluation",
        "public_sota_status": "passed_level3_public_sota",
        "registered_coding_claims_passed": True,
        "registered_coding_claims": [registered_claim],
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
            "SOTA evaluation-ready state remains blocked",
            "remem is the best system; see eval/public/reports/baseline.md.",
            sota_evaluation_ready_gate,
            "SOTA/best claim",
        ),
        (
            "unknown SOTA state remains blocked",
            "remem is the best system; see eval/public/reports/baseline.md.",
            sota_unknown_gate,
            "SOTA/best claim",
        ),
        (
            "fully passed grounded SOTA claim passes",
            "remem is best on benchmark X; see eval/public/reports/baseline.md.",
            passed_gate,
            None,
        ),
        (
            "unguarded coding superiority fails",
            "remem outperforms a maintained context file on coding tasks.",
            blocked_gate,
            "coding-outcome superiority",
        ),
        (
            "matrix-ready coding claim remains blocked",
            "remem outperforms no_memory on fixture X; see eval/public/reports/baseline.md.",
            evaluation_ready_gate,
            "coding-outcome superiority",
        ),
        (
            "registered and hash-bound coding claim passes",
            "<!-- remem-claim:remem-e2e-vs-no-memory-v1 --> On the registered fixture, remem_e2e outperforms no_memory on resolved_rate. [evidence](eval/public/reports/coding-claim.json)",
            passed_gate,
            None,
        ),
        (
            "spec link cannot authorize universal overclaim",
            "remem outperforms MEMORY.md universally; see docs/specs/public-memory-benchmark/PRODUCT.md.",
            passed_gate,
            "registered claim marker",
        ),
        (
            "report link cannot authorize universal overclaim",
            "remem outperforms every maintained context file on every coding workload; see eval/public/reports/baseline.md.",
            passed_gate,
            "registered claim marker",
        ),
        (
            "correct report cannot authorize unregistered wording",
            "<!-- remem-claim:remem-e2e-vs-no-memory-v1 --> remem outperforms no_memory universally. [evidence](eval/public/reports/coding-claim.json)",
            passed_gate,
            "registry-forbidden wording",
        ),
        (
            "adjacent policy heading cannot authorize a strong claim",
            "remem outperforms every coding workload.",
            blocked_gate,
            "coding-outcome superiority",
            "Public claim policy\n\nremem outperforms every coding workload.",
        ),
        (
            "same-line policy label cannot authorize a strong claim",
            "Public claim policy: remem outperforms every coding workload.",
            blocked_gate,
            "coding-outcome superiority",
        ),
    ]

    for case in cases:
        name, text, gate, expected, *context = case
        actual = classify_violation(text, gate, context[0] if context else None)
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
