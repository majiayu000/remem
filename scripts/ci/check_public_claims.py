#!/usr/bin/env python3
"""Guard README/release surfaces against unsupported public benchmark claims."""

from __future__ import annotations

import argparse
import json
import math
import re
import subprocess
import sys
import tempfile
from pathlib import Path
from pathlib import PurePosixPath


ROOT = Path(__file__).resolve().parents[2]

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

CLAIM_MARKER_RE = re.compile(r"<!--\s*remem-claim:([a-z0-9][a-z0-9-]*)\s*-->")

RELEASE_POLICY_CONTRACT_LINES = {
    "Do not read this as a published claim that remem beats a carefully maintained",
    "do not support SOTA, broad superiority, or coding-task superiority wording.",
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


def validate_verifier_report(value: object) -> dict[str, object]:
    if not isinstance(value, dict):
        raise ValueError("verifier JSON must be an object")
    if not isinstance(value.get("passed"), bool):
        raise ValueError("verifier JSON is missing boolean passed")
    authority = value.get("authority_verdict")
    if not isinstance(authority, dict):
        raise ValueError("verifier JSON is missing authority_verdict")
    gh931 = authority.get("gh931")
    if not isinstance(gh931, dict):
        raise ValueError("authority_verdict is missing gh931")
    security = authority.get("security")
    if not isinstance(security, dict):
        raise ValueError("authority_verdict is missing security")
    policy_failure_count = security.get("policy_failure_count")
    if not isinstance(security.get("status"), str) or type(policy_failure_count) is not int:
        raise ValueError("authority_verdict.security is malformed")
    security_reports = security.get("reports")
    if not isinstance(security_reports, list):
        raise ValueError("authority_verdict.security is missing report summaries")
    for report in security_reports:
        if not isinstance(report, dict) or not isinstance(report.get("status"), str):
            raise ValueError("authority_verdict.security contains a malformed report summary")
        summary = report.get("policy_summary")
        leak_rate = (
            summary.get("non_retention_leak_rate")
            if isinstance(summary, dict)
            else None
        )
        if (
            isinstance(leak_rate, bool)
            or not isinstance(leak_rate, (int, float))
            or not math.isfinite(leak_rate)
        ):
            raise ValueError("authority_verdict.security report leak summary is malformed")
    for field in ["completeness", "stop_loss"]:
        if not isinstance(gh931.get(field), dict):
            raise ValueError(f"authority_verdict.gh931 is missing {field}")
    claims = gh931.get("claims")
    if not isinstance(claims, list):
        raise ValueError("authority_verdict.gh931 is missing claims")
    for claim in claims:
        if not isinstance(claim, dict) or not isinstance(claim.get("id"), str):
            raise ValueError("authority_verdict.gh931 contains a malformed claim")
        for field in ["allowed_wording", "forbidden_wording"]:
            wording = claim.get(field)
            if not isinstance(wording, list) or not all(
                isinstance(item, str) and item for item in wording
            ):
                raise ValueError(f"authority_verdict.gh931 claim has invalid {field}")
    report = gh931.get("report")
    if report is not None and not isinstance(report, dict):
        raise ValueError("authority_verdict.gh931 report binding is malformed")
    return value


def load_verifier_report(path: Path) -> dict[str, object]:
    try:
        raw = path.read_text(encoding="utf-8")
    except OSError as error:
        raise ValueError(f"cannot read verifier verdict {path}: {error}") from error
    try:
        return validate_verifier_report(json.loads(raw))
    except json.JSONDecodeError as error:
        raise ValueError(f"verifier verdict is not valid JSON: {path}: {error}") from error


def acquire_verifier_report(verdict_path: Path | None) -> dict[str, object]:
    if verdict_path is not None:
        return load_verifier_report(verdict_path)
    with tempfile.TemporaryDirectory(prefix="remem-public-claims-") as directory:
        output = Path(directory) / "bench-verify.json"
        subprocess.run(
            [
                "cargo",
                "run",
                "--locked",
                "--",
                "bench",
                "verify",
                "--root",
                "eval/public",
                "--json-out",
                str(output),
            ],
            cwd=ROOT,
            check=True,
            stdout=subprocess.DEVNULL,
        )
        return load_verifier_report(output)


def line_is_closed_policy_contract(text: str, surface_path: str | None) -> bool:
    return (
        surface_path == "docs/release-lifecycle.md"
        and text.strip() in RELEASE_POLICY_CONTRACT_LINES
    )


def registered_coding_claim_violation(
    text: str, verifier_report: dict[str, object]
) -> str | None:
    markers = CLAIM_MARKER_RE.findall(text)
    if len(markers) != 1:
        return (
            "coding-outcome superiority claim must carry exactly one registered "
            "claim marker"
        )
    authority = verifier_report["authority_verdict"]
    assert isinstance(authority, dict)
    gh931 = authority["gh931"]
    security = authority["security"]
    assert isinstance(gh931, dict)
    assert isinstance(security, dict)
    completeness = gh931["completeness"]
    stop_loss = gh931["stop_loss"]
    claims = gh931["claims"]
    security_reports = security["reports"]
    assert isinstance(completeness, dict)
    assert isinstance(stop_loss, dict)
    assert isinstance(claims, list)
    assert isinstance(security_reports, list)
    claim = next(
        (
            candidate
            for candidate in claims
            if isinstance(candidate, dict) and candidate.get("id") == markers[0]
        ),
        None,
    )
    if not isinstance(claim, dict):
        return "coding claim marker is not backed by a recomputed verdict claim"
    if not (
        verifier_report["passed"] is True
        and authority.get("status") == "PASS"
        and security.get("status") == "PASS"
        and security.get("policy_failure_count") == 0
        and bool(security_reports)
        and all(
            isinstance(report, dict)
            and report.get("status") == "PASS"
            and isinstance(report.get("policy_summary"), dict)
            and report["policy_summary"].get("non_retention_leak_rate") == 0.0
            for report in security_reports
        )
        and gh931.get("status") == "PASS"
        and completeness.get("complete") is True
        and completeness.get("attempts_ready") is True
        and stop_loss.get("status") == "PASS"
        and claim.get("status") == "PASS"
    ):
        return "coding-outcome superiority claim lacks recomputed authority verdict PASS"
    forbidden = claim.get("forbidden_wording")
    if not isinstance(forbidden, list):
        return "recomputed coding claim has no forbidden-wording policy"
    lowered = text.casefold()
    if any(isinstance(phrase, str) and phrase.casefold() in lowered for phrase in forbidden):
        return "coding claim contains verdict-forbidden wording"
    report = gh931.get("report")
    allowed = claim.get("allowed_wording")
    if not isinstance(report, dict) or not isinstance(allowed, list):
        return "recomputed coding claim authority is incomplete"
    report_path = report.get("path")
    report_hash = report.get("sha256")
    if (
        not isinstance(report_path, str)
        or not isinstance(report_hash, str)
        or re.fullmatch(r"[0-9a-f]{64}", report_hash) is None
    ):
        return "recomputed coding claim has no exact report path/hash binding"
    relative = PurePosixPath(report_path)
    if relative.is_absolute() or ".." in relative.parts or not relative.parts:
        return "recomputed coding claim report path is invalid"
    public_report_path = PurePosixPath("eval/public", *relative.parts).as_posix()
    expected_lines = {
        f"<!-- remem-claim:{markers[0]} --> {wording} [evidence]({public_report_path})"
        for wording in allowed
        if isinstance(wording, str)
    }
    if text.strip() not in expected_lines:
        return (
            "coding claim must exactly match verdict allowed_wording and its "
            "verifier-bound report path"
        )
    return None


def classify_violation(
    text: str,
    verifier_report: dict[str, object],
    _context: str | None = None,
    surface_path: str | None = None,
) -> str | None:
    if not STRONG_CLAIM_RE.search(text):
        return None
    # Authorization is line-local. Adjacent headings and prose cannot turn an
    # otherwise unsupported claim into policy or negative wording.
    if line_is_closed_policy_contract(text, surface_path):
        return None

    if SOTA_CLAIM_RE.search(text):
        return "SOTA/best claim lacks independent Level 3 authority"

    if CODING_CLAIM_RE.search(text):
        return registered_coding_claim_violation(text, verifier_report)

    return "strong public claim is not grounded in an approved report"


def check_surfaces(verifier_report: dict[str, object]) -> list[str]:
    failures: list[str] = []
    for rel_path in CLAIM_SURFACES:
        path = ROOT / rel_path
        if not path.is_file():
            die(f"missing public claim surface: {rel_path}")
        lines = path.read_text(encoding="utf-8").splitlines()
        for index, line in enumerate(lines):
            context = "\n".join(lines[max(0, index - 2) : index + 2])
            reason = classify_violation(line, verifier_report, context, rel_path)
            if reason:
                failures.append(f"{rel_path}:{index + 1}: {reason}: {line.strip()}")
    return failures


def run_self_test() -> int:
    claim_id = "remem-e2e-vs-no-memory-v1"
    allowed_wording = (
        "On the registered fixture, remem_e2e outperforms no_memory on resolved_rate."
    )

    def verifier_report(
        *,
        verifier_passed: bool = True,
        gh931_status: str = "PASS",
        claim_status: str = "PASS",
        stop_loss_status: str = "PASS",
        security_status: str = "PASS",
        security_policy_failure_count: int = 0,
    ) -> dict[str, object]:
        return {
            "passed": verifier_passed,
            "authority_verdict": {
                "status": "PASS",
                "security": {
                    "status": security_status,
                    "policy_failure_count": security_policy_failure_count,
                    "reports": [
                        {
                            "status": "PASS",
                            "policy_summary": {"non_retention_leak_rate": 0.0},
                        }
                    ],
                },
                "gh931": {
                    "status": gh931_status,
                    "registry": {
                        "declared_statuses": ["PASS", "PASS", "PASS"],
                    },
                    "report": {
                        "path": "coding/reports/coding-claim.json",
                        "sha256": "1" * 64,
                    },
                    "completeness": {
                        "complete": True,
                        "attempts_ready": True,
                    },
                    "stop_loss": {"status": stop_loss_status},
                    "claims": [
                        {
                            "id": claim_id,
                            "status": claim_status,
                            "declared_registry_status": "PASS",
                            "allowed_wording": [allowed_wording],
                            "forbidden_wording": [
                                "universally",
                                "every coding workload",
                            ],
                            "supporting_report": {
                                "path": "eval/public/reports/registry-declared.json",
                                "sha256": "2" * 64,
                            },
                        }
                    ],
                }
            },
        }

    blocked_verdict = verifier_report(gh931_status="INSUFFICIENT", claim_status="INSUFFICIENT")
    passed_verdict = verifier_report()
    tampered_registry_verdict = verifier_report(claim_status="INSUFFICIENT")
    security_failed_verdict = verifier_report(security_status="FAIL")
    security_policy_failed_verdict = verifier_report(security_policy_failure_count=1)
    security_missing_summaries_verdict = verifier_report()
    security_missing_summaries_verdict["authority_verdict"]["security"]["reports"] = []
    security_nonzero_leak_verdict = verifier_report()
    security_nonzero_leak_verdict["authority_verdict"]["security"]["reports"][0][
        "policy_summary"
    ]["non_retention_leak_rate"] = 0.25
    security_malformed_leak_verdict = verifier_report()
    security_malformed_leak_verdict["authority_verdict"]["security"]["reports"][0][
        "policy_summary"
    ]["non_retention_leak_rate"] = "zero"
    stale_security_verdict = verifier_report()
    stale_security_verdict["authority_verdict"]["status"] = "INSUFFICIENT"
    for report in [
        blocked_verdict,
        passed_verdict,
        tampered_registry_verdict,
        security_failed_verdict,
        security_policy_failed_verdict,
        security_missing_summaries_verdict,
        security_nonzero_leak_verdict,
        stale_security_verdict,
    ]:
        validate_verifier_report(report)
    try:
        validate_verifier_report(security_malformed_leak_verdict)
    except ValueError:
        pass
    else:
        print("self-test failed: malformed security leak summary was accepted", file=sys.stderr)
        return 1

    copied_policy_line = "- `remem_e2e` beats `no_memory` on resolved rate by at least 10 percentage"
    copied_policy_violation = classify_violation(
        copied_policy_line,
        blocked_verdict,
        surface_path="README.md",
    )
    if copied_policy_violation is None:
        print("self-test failed: copied policy line bypassed README claim checks", file=sys.stderr)
        return 1

    cases = [
        (
            "unregistered negative SOTA wording fails closed",
            "README and release wording must not claim SOTA from this report.",
            blocked_verdict,
            "SOTA/best claim",
        ),
        (
            "policy wording passes",
            "Level 2 allowed claim requires the public claim policy gate.",
            blocked_verdict,
            None,
        ),
        (
            "unguarded SOTA fails",
            "remem is the best state-of-the-art memory system.",
            blocked_verdict,
            "SOTA/best claim",
        ),
        (
            "GH931 PASS does not invent Level 3 authority",
            "remem is the best system; see eval/public/reports/baseline.md.",
            passed_verdict,
            "SOTA/best claim",
        ),
        (
            "unguarded coding superiority fails",
            "remem outperforms a maintained context file on coding tasks.",
            blocked_verdict,
            "coding-outcome superiority",
        ),
        (
            "registry PASS and supporting report cannot authorize an insufficient claim",
            f"<!-- remem-claim:{claim_id} --> {allowed_wording} [evidence](eval/public/coding/reports/coding-claim.json)",
            tampered_registry_verdict,
            "coding-outcome superiority",
        ),
        (
            "recomputed claim uses verifier report binding and verdict wording policy",
            f"<!-- remem-claim:{claim_id} --> {allowed_wording} [evidence](eval/public/coding/reports/coding-claim.json)",
            passed_verdict,
            None,
        ),
        (
            "security authority failure blocks an otherwise passing coding claim",
            f"<!-- remem-claim:{claim_id} --> {allowed_wording} [evidence](eval/public/coding/reports/coding-claim.json)",
            security_failed_verdict,
            "coding-outcome superiority",
        ),
        (
            "security policy failure count blocks an otherwise passing coding claim",
            f"<!-- remem-claim:{claim_id} --> {allowed_wording} [evidence](eval/public/coding/reports/coding-claim.json)",
            security_policy_failed_verdict,
            "coding-outcome superiority",
        ),
        (
            "missing security report summaries block an otherwise passing coding claim",
            f"<!-- remem-claim:{claim_id} --> {allowed_wording} [evidence](eval/public/coding/reports/coding-claim.json)",
            security_missing_summaries_verdict,
            "coding-outcome superiority",
        ),
        (
            "nonzero non-retention leak rate blocks an otherwise passing coding claim",
            f"<!-- remem-claim:{claim_id} --> {allowed_wording} [evidence](eval/public/coding/reports/coding-claim.json)",
            security_nonzero_leak_verdict,
            "coding-outcome superiority",
        ),
        (
            "stale security evidence cannot authorize an otherwise passing coding claim",
            f"<!-- remem-claim:{claim_id} --> {allowed_wording} [evidence](eval/public/coding/reports/coding-claim.json)",
            stale_security_verdict,
            "coding-outcome superiority",
        ),
        (
            "spec link cannot authorize universal overclaim",
            "remem outperforms MEMORY.md universally; see docs/specs/public-memory-benchmark/PRODUCT.md.",
            passed_verdict,
            "registered claim marker",
        ),
        (
            "report link cannot authorize universal overclaim",
            "remem outperforms every maintained context file on every coding workload; see eval/public/reports/baseline.md.",
            passed_verdict,
            "registered claim marker",
        ),
        (
            "correct report cannot authorize unregistered wording",
            f"<!-- remem-claim:{claim_id} --> remem outperforms no_memory universally. [evidence](eval/public/coding/reports/coding-claim.json)",
            passed_verdict,
            "verdict-forbidden wording",
        ),
        (
            "adjacent policy heading cannot authorize a strong claim",
            "remem outperforms every coding workload.",
            blocked_verdict,
            "coding-outcome superiority",
            "Public claim policy\n\nremem outperforms every coding workload.",
        ),
        (
            "same-line policy label cannot authorize a strong claim",
            "Public claim policy: remem outperforms every coding workload.",
            blocked_verdict,
            "coding-outcome superiority",
        ),
        (
            "partial negation cannot hide a SOTA overclaim",
            "remem does not merely outperform no_memory; it is the best memory system.",
            blocked_verdict,
            "SOTA/best claim",
        ),
        (
            "competitor-subject negation cannot authorize remem superiority",
            "Competitors cannot outperform remem on coding tasks.",
            blocked_verdict,
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

    for malformed in [{}, {"passed": True}, {"passed": True, "authority_verdict": {}}]:
        try:
            validate_verifier_report(malformed)
        except ValueError:
            pass
        else:
            print("self-test failed: malformed verifier verdict was accepted", file=sys.stderr)
            return 1
    with tempfile.TemporaryDirectory(prefix="remem-public-claims-self-test-") as directory:
        missing = Path(directory) / "missing.json"
        try:
            load_verifier_report(missing)
        except ValueError as error:
            if "cannot read verifier verdict" not in str(error):
                print(f"self-test failed: unclear missing verdict error: {error}", file=sys.stderr)
                return 1
        else:
            print("self-test failed: missing verifier verdict was accepted", file=sys.stderr)
            return 1
    print("public claims check self-test: ok")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Fail on unsupported strong public benchmark claims."
    )
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument(
        "--verdict",
        type=Path,
        help="read an existing bench verify JSON instead of invoking Rust",
    )
    args = parser.parse_args()

    if args.self_test:
        return run_self_test()
    try:
        verifier_report = acquire_verifier_report(args.verdict)
    except (ValueError, OSError, subprocess.CalledProcessError) as error:
        die(f"cannot acquire runtime authority verdict: {error}")
    failures = check_surfaces(verifier_report)
    if failures:
        for failure in failures:
            print(f"FAIL: {failure}", file=sys.stderr)
        print(
            "Public claim surfaces may only make SOTA, best, beats, "
            "outperforms, or coding-superiority claims when the relevant "
            "runtime authority verdict has passed and the line uses its exact wording "
            "policy and report binding.",
            file=sys.stderr,
        )
        return 1
    print("public claims check: ok")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
