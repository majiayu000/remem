#!/usr/bin/env python3
"""Enforce the GH969 module dependency-direction no-expansion contract."""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
import tempfile
from datetime import date
from pathlib import Path

from module_dependency_discovery import DiscoveryError, ScanResult, Site, scan_repository


ROOT = Path(__file__).resolve().parents[2]
BASELINE_PATH = Path("docs/specs/GH969/module-dependency-baseline.json")
SCHEMA_VERSION = 1
SITE_KEYS = {"path", "kind", "signature", "occurrence"}
EXCEPTION_KEYS = {
    "source",
    "target",
    "site",
    "owner",
    "rationale",
    "tracking_issue",
    "decision_date",
}
SCC_EXCEPTION_KEYS = {
    "from_size",
    "to_size",
    "roots",
    "owner",
    "rationale",
    "tracking_issue",
    "decision_date",
}


class GuardError(RuntimeError):
    """Raised when the dependency contract or baseline is violated."""


def _run(arguments: list[str]) -> subprocess.CompletedProcess[str]:
    return subprocess.run(arguments, cwd=ROOT, text=True, capture_output=True, check=False)


def _site_key(value: dict[str, object]) -> tuple[str, str, str, int]:
    if set(value) != SITE_KEYS:
        raise GuardError(f"site must contain exactly {sorted(SITE_KEYS)}")
    path = value["path"]
    kind = value["kind"]
    signature = value["signature"]
    occurrence = value["occurrence"]
    if not all(isinstance(item, str) and item for item in (path, kind, signature)):
        raise GuardError("site path, kind, and signature must be non-empty strings")
    if not isinstance(occurrence, int) or occurrence < 1:
        raise GuardError("site occurrence must be a positive integer")
    return (path, kind, signature, occurrence)


def _metadata(record: dict[str, object], *, label: str) -> None:
    for field in ("owner", "rationale", "tracking_issue", "decision_date"):
        if not isinstance(record[field], str) or not str(record[field]).strip():
            raise GuardError(f"{label} {field} must be a non-empty string")
    if not re.fullmatch(r"(?:#[0-9]+|https://github\.com/[^/]+/[^/]+/issues/[0-9]+)", str(record["tracking_issue"])):
        raise GuardError(f"{label} tracking_issue must be #N or a GitHub issue URL")
    try:
        decision = date.fromisoformat(str(record["decision_date"]))
    except ValueError as error:
        raise GuardError(f"{label} decision_date must be YYYY-MM-DD") from error
    if decision < date.today():
        raise GuardError(f"{label} decision_date is overdue: {decision.isoformat()}")


def _reverse_map(scan: ScanResult) -> dict[tuple[str, str], dict[tuple[str, str, str, int], Site]]:
    result: dict[tuple[str, str], dict[tuple[str, str, str, int], Site]] = {}
    for site in scan.reverse_sites():
        result.setdefault((site.source, site.target), {})[site.key()] = site
    return result


def baseline_from_scan(scan: ScanResult, base_commit: str) -> dict[str, object]:
    reverse = _reverse_map(scan)
    return {
        "schema_version": SCHEMA_VERSION,
        "bootstrap_base_commit": base_commit,
        "accepted_reverse_edges": [
            {
                "source": source,
                "target": target,
                "sites": [site.baseline_value() for site in sorted(sites.values(), key=lambda item: item.key())],
            }
            for (source, target), sites in sorted(reverse.items())
        ],
        "largest_cyclic_component": {
            "size": len(scan.largest_cyclic_component),
            "roots": list(scan.largest_cyclic_component),
        },
        "exceptions": [],
        "scc_exceptions": [],
    }


def _baseline_sets(
    baseline: dict[str, object],
) -> tuple[
    set[tuple[str, str, tuple[str, str, str, int]]],
    set[tuple[str, str, tuple[str, str, str, int]]],
]:
    required = {
        "schema_version",
        "bootstrap_base_commit",
        "accepted_reverse_edges",
        "largest_cyclic_component",
        "exceptions",
        "scc_exceptions",
    }
    if set(baseline) != required:
        raise GuardError(f"baseline must contain exactly {sorted(required)}")
    if baseline["schema_version"] != SCHEMA_VERSION:
        raise GuardError(f"unsupported baseline schema_version {baseline['schema_version']!r}")
    if not re.fullmatch(r"[0-9a-f]{40}", str(baseline["bootstrap_base_commit"])):
        raise GuardError("bootstrap_base_commit must be a full lowercase Git SHA")

    accepted: set[tuple[str, str, tuple[str, str, str, int]]] = set()
    edges = baseline["accepted_reverse_edges"]
    if not isinstance(edges, list):
        raise GuardError("accepted_reverse_edges must be an array")
    for edge in edges:
        if not isinstance(edge, dict) or set(edge) != {"source", "target", "sites"}:
            raise GuardError("accepted reverse edge must contain source, target, and sites")
        source, target, sites = edge["source"], edge["target"], edge["sites"]
        if not isinstance(source, str) or not isinstance(target, str) or not isinstance(sites, list) or not sites:
            raise GuardError("accepted reverse edge identity and sites must be non-empty")
        for site in sites:
            if not isinstance(site, dict):
                raise GuardError("accepted reverse site must be an object")
            value = (source, target, _site_key(site))
            if value in accepted:
                raise GuardError(f"duplicate accepted reverse site: {value}")
            accepted.add(value)

    exceptions: set[tuple[str, str, tuple[str, str, str, int]]] = set()
    raw_exceptions = baseline["exceptions"]
    if not isinstance(raw_exceptions, list):
        raise GuardError("exceptions must be an array")
    for exception in raw_exceptions:
        if not isinstance(exception, dict) or set(exception) != EXCEPTION_KEYS:
            raise GuardError(f"exception must contain exactly {sorted(EXCEPTION_KEYS)}")
        _metadata(exception, label="dependency exception")
        if not isinstance(exception["site"], dict):
            raise GuardError("dependency exception site must be an object")
        value = (str(exception["source"]), str(exception["target"]), _site_key(exception["site"]))
        if value in exceptions or value in accepted:
            raise GuardError(f"duplicate or already-accepted dependency exception: {value}")
        exceptions.add(value)

    scc_exceptions = baseline["scc_exceptions"]
    if not isinstance(scc_exceptions, list):
        raise GuardError("scc_exceptions must be an array")
    for exception in scc_exceptions:
        if not isinstance(exception, dict) or set(exception) != SCC_EXCEPTION_KEYS:
            raise GuardError(f"SCC exception must contain exactly {sorted(SCC_EXCEPTION_KEYS)}")
        _metadata(exception, label="SCC exception")
        if not isinstance(exception["from_size"], int) or not isinstance(exception["to_size"], int):
            raise GuardError("SCC exception sizes must be integers")
        roots = exception["roots"]
        if not isinstance(roots, list) or not roots or any(not isinstance(root, str) for root in roots):
            raise GuardError("SCC exception roots must be a non-empty string array")
    return accepted, exceptions


def _largest(baseline: dict[str, object]) -> tuple[int, tuple[str, ...]]:
    value = baseline["largest_cyclic_component"]
    if not isinstance(value, dict) or set(value) != {"size", "roots"}:
        raise GuardError("largest_cyclic_component must contain size and roots")
    size, roots = value["size"], value["roots"]
    if not isinstance(size, int) or size < 0 or not isinstance(roots, list):
        raise GuardError("largest cyclic component has invalid size or roots")
    if any(not isinstance(root, str) for root in roots) or roots != sorted(set(roots)) or size != len(roots):
        raise GuardError("largest cyclic component roots must be sorted, unique, and match size")
    return size, tuple(roots)


def validate_current(scan: ScanResult, baseline: dict[str, object]) -> None:
    accepted, exceptions = _baseline_sets(baseline)
    current = {
        (site.source, site.target, site.key())
        for site in scan.reverse_sites()
    }
    missing = sorted(current - accepted - exceptions)
    stale = sorted((accepted | exceptions) - current)
    errors: list[str] = []
    current_sites = {
        (site.source, site.target, site.key()): site
        for site in scan.reverse_sites()
    }
    for value in missing:
        site = current_sites[value]
        source_layer = scan.root_layers[site.source][1]
        target_layer = scan.root_layers[site.target][1]
        errors.append(
            f"new reverse dependency {site.source} ({source_layer}) -> {site.target} ({target_layer}) "
            f"at {site.path}:{site.line} [{site.signature}]; move the dependency inward or add a "
            "reviewed temporary exception with owner, rationale, issue, and decision date"
        )
    for source, target, site in stale:
        errors.append(
            f"baseline site no longer exists for {source} -> {target}: {site}; remove it so the accepted baseline shrinks"
        )
    expected_largest = (len(scan.largest_cyclic_component), scan.largest_cyclic_component)
    if _largest(baseline) != expected_largest:
        errors.append(
            "largest_cyclic_component is stale: baseline "
            f"{_largest(baseline)}, discovered {expected_largest}; update it to the measured value"
        )
    if errors:
        raise GuardError("\n  - ".join(["module dependency guard failed:", *errors]))


def _manifest_at(ref: str) -> dict[str, object] | None:
    result = _run(["git", "show", f"{ref}:{BASELINE_PATH.as_posix()}"])
    if result.returncode != 0:
        return None
    value = json.loads(result.stdout)
    if not isinstance(value, dict):
        raise GuardError(f"module dependency baseline at {ref} is not an object")
    return value


def _resolve_commit(ref: str) -> str:
    result = _run(["git", "rev-parse", f"{ref}^{{commit}}"])
    if result.returncode != 0:
        raise GuardError(f"cannot resolve base ref {ref}: {result.stderr.strip()}")
    return result.stdout.strip()


def validate_history(
    scan: ScanResult,
    baseline: dict[str, object],
    base: str,
) -> None:
    previous = _manifest_at(base)
    if previous is None:
        base_commit = _resolve_commit(base)
        expected = baseline_from_scan(scan, base_commit)
        if baseline != expected:
            raise GuardError(
                "initial baseline must be the exact scanner output, contain no exceptions, and bind the PR base commit"
            )
        product_diff = _run(["git", "diff", "--quiet", base, "HEAD", "--", "Cargo.toml", "src"])
        if product_diff.returncode != 0:
            raise GuardError("initial baseline cannot bootstrap across Rust product-source changes")
        return

    validate_transition(scan, baseline, previous)


def validate_transition(
    scan: ScanResult,
    baseline: dict[str, object],
    previous: dict[str, object],
) -> None:
    """Authenticate a non-bootstrap baseline change against its predecessor."""
    current_accepted, _ = _baseline_sets(baseline)

    previous_accepted, _ = _baseline_sets(previous)
    added_accepted = current_accepted - previous_accepted
    if added_accepted:
        raise GuardError(
            "accepted reverse-dependency baseline cannot grow; new sites require reviewed temporary exceptions: "
            + repr(sorted(added_accepted))
        )
    previous_size, _ = _largest(previous)
    current_size, current_roots = _largest(baseline)
    if current_size > previous_size:
        new_scc_exceptions = [
            item
            for item in baseline["scc_exceptions"]
            if item not in previous["scc_exceptions"]
        ]
        authorized = any(
            item["from_size"] == previous_size
            and item["to_size"] == current_size
            and tuple(item["roots"]) == current_roots
            for item in new_scc_exceptions
        )
        if not authorized:
            raise GuardError(
                f"largest cyclic component grew from {previous_size} to {current_size} without an exact reviewed SCC exception"
            )


def print_current(scan: ScanResult) -> None:
    grouped = _reverse_map(scan)
    print(
        "module dependency guard: ok "
        f"({sum(len(sites) for sites in grouped.values())} accepted reverse sites across "
        f"{len(grouped)} edges; largest cyclic component size "
        f"{len(scan.largest_cyclic_component)})"
    )
    for (source, target), sites in sorted(grouped.items()):
        locations = ", ".join(
            f"{site.path}:{site.line}" for site in sorted(sites.values(), key=lambda item: (item.path, item.line))
        )
        print(f"  accepted {source} -> {target}: {locations}")
    if scan.largest_cyclic_component:
        print("  largest cycle: " + ", ".join(scan.largest_cyclic_component))


def _write_fixture(root: Path, relative: str, value: str) -> None:
    path = root / relative
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(value, encoding="utf-8")


def self_test() -> None:
    layers = {
        "inner": (0, "foundation/domain"),
        "middle": (1, "storage"),
        "outer": (2, "application"),
        "diag": (3, "evidence/diagnostics"),
    }
    with tempfile.TemporaryDirectory(prefix="module-dependency-self-test-") as raw_tmp:
        root = Path(raw_tmp)
        _write_fixture(root, "src/lib.rs", "mod inner;\nmod middle;\nmod outer;\nmod diag;\n")
        _write_fixture(
            root,
            "src/inner.rs",
            """use crate::outer::Thing;
// crate::outer::Ignored
const TEXT: &str = r#"crate::outer::Ignored"#;
#[cfg(test)]
use crate::outer::TestOnly;
#[cfg(test)]
mod tests;
mod prod_tests;
mod nested { pub use super::super::outer::Other; }
""",
        )
        _write_fixture(root, "src/inner/tests.rs", "use crate::outer::Ignored;\n")
        _write_fixture(root, "src/inner/prod_tests.rs", "use crate::outer::Visible;\n")
        _write_fixture(root, "src/middle.rs", "pub use crate::inner::{self, Value};\n")
        _write_fixture(root, "src/outer.rs", "use crate::{inner, middle::{self, Value}};\n")
        _write_fixture(root, "src/diag.rs", "use crate::outer;\n")
        scan = scan_repository(root, configured_layers=layers)
        reverse = scan.reverse_sites()
        signatures = {(site.path, site.kind, site.signature) for site in reverse}
        expected_paths = {"src/inner.rs", "src/inner/prod_tests.rs"}
        if {site.path for site in reverse} != expected_paths or len(reverse) != 3:
            raise GuardError(f"scanner form/test masking self-test failed: {signatures}")
        if len(scan.largest_cyclic_component) != 3:
            raise GuardError(f"cycle discovery self-test failed: {scan.cyclic_components}")

        baseline = baseline_from_scan(scan, "0" * 40)
        validate_current(scan, baseline)
        _write_fixture(root, "src/inner.rs", (root / "src/inner.rs").read_text() + "crate::outer::Added;\n")
        expanded = scan_repository(root, configured_layers=layers)
        try:
            validate_current(expanded, baseline)
        except GuardError as error:
            if "new reverse dependency inner" not in str(error):
                raise
        else:
            raise GuardError("new reverse source site did not fail")

        expanded_baseline = baseline_from_scan(expanded, "0" * 40)
        try:
            validate_transition(expanded, expanded_baseline, baseline)
        except GuardError as error:
            if "accepted reverse-dependency baseline cannot grow" not in str(error):
                raise
        else:
            raise GuardError("accepted reverse-dependency baseline growth did not fail")

        _write_fixture(
            root,
            "src/inner.rs",
            (root / "src/inner.rs").read_text().replace("crate::outer::Added;\n", ""),
        )
        _write_fixture(root, "src/outer.rs", "use crate::{inner, middle};\nuse crate::diag;\n")
        grown_scan = scan_repository(root, configured_layers=layers)
        grown = baseline_from_scan(grown_scan, "0" * 40)
        new_edge = next(
            edge
            for edge in grown["accepted_reverse_edges"]
            if edge["source"] == "outer" and edge["target"] == "diag"
        )
        grown["accepted_reverse_edges"].remove(new_edge)
        grown["exceptions"].append(
            {
                "source": "outer",
                "target": "diag",
                "site": new_edge["sites"][0],
                "owner": "architecture",
                "rationale": "self-test cycle-growth exception",
                "tracking_issue": "#1044",
                "decision_date": "2099-12-31",
            }
        )
        validate_current(grown_scan, grown)
        try:
            validate_transition(grown_scan, grown, baseline)
        except GuardError as error:
            if "largest cyclic component grew" not in str(error):
                raise
        else:
            raise GuardError("largest cyclic component growth did not fail")
    print("module dependency guard self-test: ok")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--base", help="PR base ref or prior push SHA for baseline authentication")
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument(
        "--bootstrap",
        metavar="BASE_REF",
        help="write the one-time exact baseline for a base that has no committed baseline",
    )
    args = parser.parse_args()
    try:
        if args.self_test:
            self_test()
            return 0
        scan = scan_repository(ROOT)
        if args.bootstrap:
            if (ROOT / BASELINE_PATH).exists():
                raise GuardError("refusing bootstrap because the baseline already exists")
            baseline = baseline_from_scan(scan, _resolve_commit(args.bootstrap))
            (ROOT / BASELINE_PATH).write_text(json.dumps(baseline, indent=2) + "\n", encoding="utf-8")
            print(f"wrote exact bootstrap baseline to {BASELINE_PATH}")
            return 0
        baseline = json.loads((ROOT / BASELINE_PATH).read_text(encoding="utf-8"))
        if not isinstance(baseline, dict):
            raise GuardError("module dependency baseline must be a JSON object")
        validate_current(scan, baseline)
        if args.base:
            validate_history(scan, baseline, args.base)
        print_current(scan)
        return 0
    except (DiscoveryError, GuardError, OSError, json.JSONDecodeError) as error:
        print(f"FAIL: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
