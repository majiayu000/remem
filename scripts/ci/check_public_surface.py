#!/usr/bin/env python3
"""Check public release and discoverability surfaces that CI can prove locally."""

from __future__ import annotations

import argparse
import copy
import json
import re
import sys
import tempfile
from collections import Counter, defaultdict
from datetime import date
from pathlib import Path

from surface_lifecycle_discovery import (
    ROW_STATUS,
    build_manifest,
    discover_all,
    discover_cli_commands,
    discover_mcp_tools,
    discover_product_row_statuses,
    lifecycle_record,
)


ROOT = Path(__file__).resolve().parents[2]

README_BADGES = [
    "actions/workflows/ci.yml/badge.svg",
    "img.shields.io/github/v/release/majiayu000/remem",
    "img.shields.io/crates/v/remem-ai",
    "img.shields.io/npm/v/%40remem-ai%2Fremem",
    "License-MIT",
]

README_REQUIRED_TEXT = [
    "brew install majiayu000/tap/remem",
    "npm install -g @remem-ai/remem",
    "cargo install remem-ai --bin remem",
    "remem doctor",
    "remem search \"last decision\"",
    "GitHub Releases: prebuilt binaries",
]

ROOT_REQUIRED_FILES = [
    "README.md",
    "README.zh-CN.md",
    "CHANGELOG.md",
    "CODE_OF_CONDUCT.md",
    "CONTRIBUTING.md",
    "SECURITY.md",
    "LICENSE",
    "assets/remem-demo.gif",
    "assets/social-preview.svg",
    "docs/release-lifecycle.md",
    "docs/maintenance/file-size-debt.md",
]

SITE_PAGES = [
    "site/index.html",
    "site/claude-code-memory/index.html",
    "site/codex-memory/index.html",
    "site/mcp-memory-server/index.html",
    "site/compare/built-in-memory/index.html",
]

MANIFEST_PATH = ROOT / "docs/specs/GH969/surface-manifest.json"
REQUIRED_RECORD_FIELDS = {
    "id", "surface_kind", "owner", "status", "public_entry_points",
    "real_callers", "default_state", "spec_refs", "eval_commands",
    "compatibility", "rollback", "decision_due",
}
DISCOVERED_KINDS = {"rust_export", "mcp_tool", "rest_route", "cli_command", "default_feature"}
SPECIAL_KINDS = {
    "runtime_component", "mcp_parameter", "recovery_path", "compatibility_path",
    "offline_harness", "spec_contract",
}
DATED_STATUSES = {"staged", "experimental", "deprecated", "spec-only"}
KNOWN_STATUSES = {"staged", "production", "experimental", "recovery-only", "deprecated", "spec-only"}


def fail(message: str) -> None:
    print(f"FAIL: {message}", file=sys.stderr)
    sys.exit(1)


def require_file(path: str) -> None:
    if not (ROOT / path).is_file():
        fail(f"missing {path}")


def require_contains(label: str, text: str, needle: str) -> None:
    if needle not in text:
        fail(f"{label} is missing {needle!r}")


def require_site_page(path: str) -> None:
    full = ROOT / path
    require_file(path)
    text = full.read_text(encoding="utf-8")
    for needle in [
        "<title>",
        'name="description"',
        'rel="canonical"',
        'property="og:title"',
        'name="twitter:card"',
        'name="robots" content="index,follow"',
    ]:
        require_contains(path, text, needle)
    if len(re.findall(r"<h1\b", text)) != 1:
        fail(f"{path} must contain exactly one h1")


def _string_list(value: object) -> bool:
    return isinstance(value, list) and all(isinstance(item, str) and item.strip() for item in value)


def _declared_files(artifacts: dict[str, object]) -> set[str]:
    declared: set[str] = set()
    for category in ("executables", "schemas", "fixtures", "data", "documents"):
        values = artifacts.get(category)
        if _string_list(values):
            declared.update(values)  # type: ignore[arg-type]
    return declared


def _production_rust_sources(root: Path) -> list[tuple[str, str]]:
    sources: list[tuple[str, str]] = []
    for path in sorted((root / "src").rglob("*.rs")):
        relative = path.relative_to(root).as_posix()
        if (
            "tests" in path.parts
            or path.name == "test_support.rs"
            or path.stem.startswith("tests")
            or path.stem.endswith("_tests")
        ):
            continue
        text = path.read_text(encoding="utf-8")
        sources.append((relative, text))
    return sources


def discover_recovery_writers(root: Path, guard: dict[str, object]) -> set[str]:
    mode = guard.get("mode")
    target = guard.get("target")
    if not isinstance(target, str) or mode not in {"sql_table_insert", "summary_job_enqueue"}:
        raise RuntimeError(f"unknown recovery writer guard {guard!r}")
    writers: set[str] = set()
    for relative, text in _production_rust_sources(root):
        sql_pattern = rf"\bINSERT\s+(?:OR\s+[A-Z]+\s+)?INTO\s+{re.escape(target)}\b"
        candidate = re.search(sql_pattern, text, re.I) if mode == "sql_table_insert" else re.search(
            r"\bJobType::Summary\b|\bINSERT\s+(?:OR\s+[A-Z]+\s+)?INTO\s+jobs\b",
            text,
            re.I,
        )
        if not candidate:
            continue
        while match := re.search(r"#\s*\[\s*cfg\s*\(\s*test\s*\)\s*\][^{;]*\{", text):
            opening = text.find("{", match.start())
            depth = 0
            for index in range(opening, len(text)):
                depth += text[index] == "{"
                depth -= text[index] == "}"
                if depth == 0:
                    text = text[: match.start()] + " " * (index + 1 - match.start()) + text[index + 1 :]
                    break
            else:
                raise RuntimeError(f"unclosed cfg(test) block in {relative}")
        if mode == "sql_table_insert" and re.search(
            sql_pattern,
            text,
            re.I,
        ):
            writers.add(relative)
        if mode == "summary_job_enqueue":
            for match in re.finditer(r"\benqueue[A-Za-z0-9_:]*\s*\(", text):
                opening = text.find("(", match.start())
                depth = 0
                for index in range(opening, len(text)):
                    depth += text[index] == "("
                    depth -= text[index] == ")"
                    if depth == 0:
                        if re.search(r"\bJobType::Summary\b", text[opening + 1 : index]):
                            writers.add(relative)
                        break
            literals = re.findall(r'"((?:\\.|[^"\\])*)"', text, re.S)
            if any(
                re.search(r"\bINSERT\s+(?:OR\s+[A-Z]+\s+)?INTO\s+jobs\b", literal, re.I)
                and re.search(r"['\"]summary['\"]", literal, re.I)
                for literal in literals
            ):
                writers.add(relative)
    return writers


def check_lifecycle_manifest(
    root: Path,
    manifest: dict[str, object],
    discovered: dict[str, set[str]],
    *,
    today: date,
    required_rows: set[str] | None = None,
    check_wording: bool = False,
) -> list[str]:
    errors: list[str] = []
    canonical_statuses = ROW_STATUS
    if check_wording:
        try:
            canonical_statuses = discover_product_row_statuses(root)
        except RuntimeError as error:
            errors.append(str(error))
        if canonical_statuses != ROW_STATUS:
            errors.append(
                "GH969 PRODUCT canonical statuses contradict discovery metadata: "
                f"product={canonical_statuses} code={ROW_STATUS}"
            )
    records = manifest.get("records")
    if manifest.get("schema_version") != 1 or not isinstance(records, list):
        return ["surface manifest must have schema_version=1 and a records array"]
    ids: Counter[str] = Counter()
    rows: set[str] = set()
    recovery_guards: dict[str, list[dict[str, object]]] = defaultdict(list)
    classified: dict[tuple[str, str], list[str]] = defaultdict(list)
    for index, raw_record in enumerate(records):
        if not isinstance(raw_record, dict):
            errors.append(f"record[{index}] must be an object")
            continue
        record = raw_record
        record_id = record.get("id")
        label = record_id if isinstance(record_id, str) else f"record[{index}]"
        if isinstance(record_id, str) and record_id:
            ids[record_id] += 1
        else:
            errors.append(f"{label}: missing non-empty id")
        missing = sorted(REQUIRED_RECORD_FIELDS - record.keys())
        if missing:
            errors.append(f"{label}: missing required fields {', '.join(missing)}")
        kind = record.get("surface_kind")
        if kind not in DISCOVERED_KINDS | SPECIAL_KINDS:
            errors.append(f"{label}: unknown surface_kind {kind!r}")
        status = record.get("status")
        if status not in KNOWN_STATUSES:
            errors.append(f"{label}: unknown lifecycle status {status!r}")
        row_name = record.get("inventory_row")
        if not isinstance(row_name, str) or row_name not in canonical_statuses:
            errors.append(f"{label}: unknown or missing canonical inventory_row {row_name!r}")
        else:
            rows.add(row_name)
            if status != canonical_statuses[row_name]:
                errors.append(f"{label}: status {status!r} contradicts canonical PRODUCT row {row_name}={canonical_statuses[row_name]!r}")
        if not isinstance(record.get("owner"), str) or not str(record.get("owner", "")).strip():
            errors.append(f"{label}: owner is required; assign the responsible source/spec owner")
        points = record.get("public_entry_points")
        if not _string_list(points):
            errors.append(f"{label}: public_entry_points must be a non-empty string array")
            points = []
        callers = record.get("real_callers")
        if not _string_list(callers):
            errors.append(f"{label}: real_callers must be a non-empty string array")
            callers = []
        if status == "production" and (not callers or all(str(item).lower().startswith("none") for item in callers)):
            errors.append(f"{label}: production surface has no real production caller")
        specs = record.get("spec_refs")
        if not _string_list(specs):
            errors.append(f"{label}: spec_refs must be a non-empty string array")
        else:
            for spec in specs:
                if not (root / spec).is_file():
                    errors.append(f"{label}: spec reference does not resolve: {spec}")
        due = record.get("decision_due")
        if status in DATED_STATUSES:
            if not isinstance(due, str):
                errors.append(f"{label}: {status} requires an ISO decision_due date")
            else:
                try:
                    parsed_due = date.fromisoformat(due)
                    if parsed_due < today:
                        errors.append(f"{label}: decision_due {due} is overdue; integrate, continue with a new date, or remove")
                except ValueError:
                    errors.append(f"{label}: invalid ISO decision_due {due!r}")
        state = str(record.get("default_state", "")).lower()
        claims_default = (
            state.startswith(("default", "required"))
            or " default path" in state
            or "default-on" in state
            or "required for normal operation" in state
        )
        explicitly_non_default = (
            "not a default" in state
            or "not required" in state
            or "separate" in state
        )
        if status == "experimental" and claims_default and not explicitly_non_default:
            errors.append(f"{label}: experimental surface claims a default production path without a separately classified production entry")
        if status == "recovery-only":
            writers = record.get("normal_writers")
            if not isinstance(writers, list) or writers:
                errors.append(f"{label}: recovery-only record must declare normal_writers=[]; new-work writers are forbidden")
            guard = record.get("writer_guard")
            if isinstance(row_name, str) and isinstance(guard, dict):
                recovery_guards[row_name].append(record)
        if kind in DISCOVERED_KINDS:
            if len(points) != 1:
                errors.append(f"{label}: {kind} records classify exactly one reachable entry point")
            for point in points:
                classified[(str(kind), point)].append(str(label))
        elif kind == "offline_harness":
            artifacts = record.get("artifacts")
            if not isinstance(artifacts, dict) or not _string_list(artifacts.get("roots")):
                errors.append(f"{label}: offline_harness requires artifacts.roots and exact categorized inventory")
            else:
                roots = artifacts["roots"]
                actual = {
                    path.relative_to(root).as_posix()
                    for declared_root in roots
                    for path in (root / declared_root).rglob("*")
                    if path.is_file()
                }
                declared = _declared_files(artifacts)
                if actual != declared:
                    errors.append(f"{label}: offline artifact inventory drift: missing={sorted(actual-declared)} stale={sorted(declared-actual)}")
                command = artifacts.get("checked_command")
                if not isinstance(command, str) or len(command.split()) < 2 or command.split()[1] not in declared:
                    errors.append(f"{label}: checked_command must invoke a declared executable")
        elif status != "spec-only":
            for point in points:
                if not (root / point).exists():
                    errors.append(f"{label}: non-spec-only entry no longer resolves: {point}")
        else:
            for point in points:
                if not (root / point).exists():
                    errors.append(f"{label}: spec-only contract no longer resolves: {point}")

    for record_id, count in sorted(ids.items()):
        if count > 1:
            errors.append(f"duplicate manifest id {record_id!r} appears {count} times")
    for kind, actual_entries in discovered.items():
        for entry in sorted(actual_entries):
            owners = classified.get((kind, entry), [])
            if not owners:
                errors.append(f"unclassified {kind} surface {entry!r}; add one lifecycle record")
            elif len(owners) > 1:
                errors.append(f"multiply classified {kind} surface {entry!r}: {', '.join(owners)}")
        for classified_kind, entry in sorted(classified):
            if classified_kind == kind and entry not in actual_entries:
                errors.append(f"stale {kind} manifest surface {entry!r}; remove it or restore the registered/exported entry")
    expected_rows = set(canonical_statuses) if required_rows is None else required_rows
    missing_rows = sorted(expected_rows - rows)
    if missing_rows:
        errors.append(f"canonical PRODUCT inventory rows missing from manifest: {', '.join(missing_rows)}")

    recovery_rows = {
        row_name for row_name, status in canonical_statuses.items()
        if status == "recovery-only" and row_name in rows
    }
    for row_name in sorted(recovery_rows):
        guards = recovery_guards.get(row_name, [])
        if len(guards) != 1:
            errors.append(f"recovery-only row {row_name!r} requires exactly one implementation writer_guard")
            continue
        guard = guards[0].get("writer_guard")
        assert isinstance(guard, dict)
        try:
            actual_writers = discover_recovery_writers(root, guard)
        except RuntimeError as error:
            errors.append(f"recovery-only row {row_name!r}: {error}")
            continue
        if actual_writers:
            errors.append(
                f"recovery-only row {row_name!r} has implementation writers forbidden by PRODUCT: {sorted(actual_writers)}"
            )

    if check_wording:
        readme = (root / "README.md").read_text(encoding="utf-8")
        for marker in ("experimental MCP `context_bundle`", "experimental `remem context-plan"):
            if marker not in readme:
                errors.append(f"README.md lifecycle wording is missing {marker!r}")
        _, titles = discover_mcp_tools(root)
        for name, title in titles.items():
            expected = "experimental" if name == "context_bundle" else "production"
            has_experimental = "experimental" in title.lower()
            if (expected == "experimental") != has_experimental:
                errors.append(f"served MCP title for {name!r} contradicts {expected}: {title!r}")
        index = (root / "docs/specs/README.md").read_text(encoding="utf-8")
        for marker in ("Context Bundle v1 implemented; API remains experimental", "v1 infrastructure; completion unimplemented"):
            if marker not in index:
                errors.append(f"docs/specs/README.md lifecycle wording is missing {marker!r}")
    return errors


def lifecycle_self_test() -> int:
    with tempfile.TemporaryDirectory(prefix="remem-surface-lifecycle-") as raw:
        root = Path(raw)
        (root / "src/mcp/server").mkdir(parents=True)
        (root / "src/api").mkdir(parents=True)
        (root / "src/cli").mkdir(parents=True)
        (root / "docs/specs/GH969").mkdir(parents=True)
        (root / "docs/specs/GH969/PRODUCT.md").write_text(
            "## Canonical Surface Inventory\n\n| Inventory row | Entry | Owner | Status |\n"
            "|---|---|---|---|\n| `fixture-row` | fixture | fixture | `production` |\n"
            "\n## Decision Gates\n",
            encoding="utf-8",
        )
        (root / "Cargo.toml").write_text('[features]\ndefault = ["eval"]\neval = []\n', encoding="utf-8")
        (root / "src/mcp/server/tool_contracts.rs").write_text(
            'struct ToolContract; const CONTRACTS: [ToolContract; 1] = [json_object("search", "Search", true, false, true, true, Schema::Search)\n];\n', encoding="utf-8"
        )
        (root / "src/api/server.rs").write_text(
            'fn build_router() { Router::new().merge(public_routes()).nest("/api/v1/admin", admin_routes()) }\n'
            'fn public_routes() { Router::new().route("/health", get(health).post(check)) }\n'
            'fn admin_routes() { Router::new().route("/check", post(check)) }\n',
            encoding="utf-8",
        )
        (root / "src/cli/types.rs").write_text(
            '#[derive(Subcommand)] enum Commands { '
            '#[command(visible_alias = "ctx")] Context, '
            '#[cfg(feature = "eval")] Eval, #[cfg(feature = "off")] Hidden, '
            'Admin { #[command(subcommand)] action: AdminAction } }\n'
            '#[derive(Subcommand)] enum AdminAction { #[command(alias = "save")] Backup }\n',
            encoding="utf-8",
        )
        doc_root = root / "doc/remem"
        (doc_root / "api").mkdir(parents=True)
        (doc_root / "index.html").write_text(
            '<a class="mod" href="api/index.html">api</a>', encoding="utf-8"
        )
        (doc_root / "api/index.html").write_text("module", encoding="utf-8")
        (doc_root / "all.html").write_text(
            '<ul class="all-items"><li><a href="api/struct.RouterInfo.html">api::RouterInfo</a></li></ul>',
            encoding="utf-8",
        )
        (doc_root / "api/struct.RouterInfo.html").write_text(
            '<span id="structfield.status"></span><div id="implementations-list">'
            '<section id="method.health"></section></div><h2 id="trait-implementations"></h2>'
            '<section id="method.clone" class="trait-impl"></section>',
            encoding="utf-8",
        )
        discovered = discover_all(root, doc_root=doc_root)
        expected = {
            "remem context", "remem ctx", "remem eval", "remem admin",
            "remem admin backup", "remem admin save", "remem help", "remem admin help",
        }
        expected_rust = {
            "remem::api", "remem::api::RouterInfo", "remem::api::RouterInfo::status",
            "remem::api::RouterInfo::health",
        }
        expected_rest = {"GET /health", "POST /health", "POST /api/v1/admin/check"}
        if (
            discovered["cli_command"] != expected
            or discovered["rest_route"] != expected_rest
            or not expected_rust.issubset(discovered["rust_export"])
            or "remem::api::RouterInfo::clone" in discovered["rust_export"]
        ):
            print(f"surface discovery self-test failed: {discovered}", file=sys.stderr)
            return 1
        (root / "Cargo.toml").write_text('[features]\ndefault = []\neval = []\n', encoding="utf-8")
        if "remem eval" in discover_cli_commands(root):
            sys.stderr.write("default-feature CLI discovery retained disabled eval command\n")
            return 1
        (root / "Cargo.toml").write_text('[features]\ndefault = ["eval"]\neval = []\n', encoding="utf-8")
        if discover_product_row_statuses(root) != {"fixture-row": "production"}:
            sys.stderr.write("PRODUCT inventory status discovery self-test failed\n")
            return 1
        (root / "src/runtime.rs").write_text(
            'fn write(conn: &Connection) { conn.execute("INSERT INTO pending_observations DEFAULT VALUES", []); }',
            encoding="utf-8",
        )
        writer_guard = {"mode": "sql_table_insert", "target": "pending_observations"}
        if discover_recovery_writers(root, writer_guard) != {"src/runtime.rs"}:
            sys.stderr.write("recovery writer discovery self-test failed\n")
            return 1
        (root / "src/runtime.rs").write_text("fn read() {}", encoding="utf-8")
        records = [
            lifecycle_record(f"{kind}:{entry}", kind, entry, "mcp-production" if kind == "mcp_tool" else "rust-library" if kind == "rust_export" else "rest-api" if kind == "rest_route" else "cli-production" if kind == "cli_command" else "deterministic-eval")
            for kind, entries in discovered.items() for entry in entries
        ]
        for item in records:
            for spec in item["spec_refs"]:
                spec_path = root / str(spec)
                spec_path.parent.mkdir(parents=True, exist_ok=True)
                if not spec_path.exists():
                    spec_path.write_text("fixture", encoding="utf-8")
        manifest: dict[str, object] = {"schema_version": 1, "records": records}
        positive_errors = check_lifecycle_manifest(
            root, manifest, discovered, today=date(2026, 8, 24), required_rows=set()
        )
        if positive_errors:
            print(f"positive lifecycle manifest self-test failed: {positive_errors}", file=sys.stderr)
            return 1

        def proves(mutator: object, needle: str) -> bool:
            candidate = copy.deepcopy(manifest)
            assert callable(mutator)
            mutator(candidate)
            return any(needle in error for error in check_lifecycle_manifest(root, candidate, discovered, today=date(2026, 8, 24), required_rows=set()))

        cases = [
            (lambda value: value["records"][0].update(status="mystery"), "unknown lifecycle status"),
            (lambda value: value["records"][0].update(owner=""), "owner is required"),
            (lambda value: value["records"].append(copy.deepcopy(value["records"][0])), "multiply classified"),
            (lambda value: value["records"].pop(0), "unclassified"),
            (lambda value: value["records"][0]["public_entry_points"].__setitem__(0, "stale"), "stale"),
            (lambda value: value["records"][0].update(status="production", real_callers=["none"]), "no real production caller"),
            (lambda value: value["records"][0].update(status="experimental", inventory_row="mcp-context-bundle", decision_due="2026-01-01"), "overdue"),
            (lambda value: value["records"][0].update(status="experimental", inventory_row="mcp-context-bundle", decision_due="2026-11-30", default_state="default path"), "without a separately classified"),
            (lambda value: value["records"][0].update(status="recovery-only", inventory_row="legacy-pending", normal_writers=["new work"]), "new-work writers are forbidden"),
            (lambda value: value["records"][0].update(spec_refs=["missing.md"]), "does not resolve"),
        ]
        failed = [needle for mutator, needle in cases if not proves(mutator, needle)]
        if failed:
            print(f"negative lifecycle self-tests failed: {failed}", file=sys.stderr)
            return 1
    print("surface lifecycle guard self-test: ok")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        return lifecycle_self_test()

    for path in ROOT_REQUIRED_FILES:
        require_file(path)

    readme = (ROOT / "README.md").read_text(encoding="utf-8")
    for needle in README_BADGES + README_REQUIRED_TEXT:
        require_contains("README.md", readme, needle)

    zh_readme = (ROOT / "README.zh-CN.md").read_text(encoding="utf-8")
    for needle in README_BADGES:
        require_contains("README.zh-CN.md", zh_readme, needle)

    robots = (ROOT / "site/robots.txt").read_text(encoding="utf-8")
    require_contains("site/robots.txt", robots, "Sitemap: https://majiayu000.github.io/remem/sitemap.xml")
    sitemap = (ROOT / "site/sitemap.xml").read_text(encoding="utf-8")
    for url in [
        "https://majiayu000.github.io/remem/",
        "https://majiayu000.github.io/remem/codex-memory/",
        "https://majiayu000.github.io/remem/claude-code-memory/",
        "https://majiayu000.github.io/remem/mcp-memory-server/",
    ]:
        require_contains("site/sitemap.xml", sitemap, url)

    for page in SITE_PAGES:
        require_site_page(page)

    codex_page = (ROOT / "site/codex-memory/index.html").read_text(encoding="utf-8")
    require_contains("site/codex-memory/index.html", codex_page, "application/ld+json")

    try:
        manifest = json.loads(MANIFEST_PATH.read_text(encoding="utf-8"))
        discovered = discover_all(ROOT)
    except (OSError, json.JSONDecodeError, RuntimeError) as exc:
        fail(f"surface lifecycle discovery could not run: {exc}")
    lifecycle_errors = check_lifecycle_manifest(
        ROOT,
        manifest,
        discovered,
        today=date.today(),
        check_wording=True,
    )
    if lifecycle_errors:
        fail("surface lifecycle guard failed:\n  - " + "\n  - ".join(lifecycle_errors))

    print("public surface and lifecycle check: ok")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
