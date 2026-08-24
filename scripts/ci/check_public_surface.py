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
    lifecycle_record,
    lifecycle_row,
)
from surface_lifecycle_evidence import (
    EXPERIMENTAL_CALLER_SYMBOLS,
    PRODUCTION_DEFAULT_GUARDS,
    build_caller_guard,
    build_default_guard,
    discover_product_rows,
    offline_categories,
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
    "canonical_entry", "evidence", "compatibility", "next_decision", "rollback", "decision_due",
}
DISCOVERED_KINDS = {
    "rust_export", "rust_target_export", "mcp_tool", "mcp_parameter",
    "rest_route", "cli_command", "default_feature",
}
SPECIAL_KINDS = {
    "runtime_component", "recovery_path", "compatibility_path",
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
    canonical_rows_override: dict[str, dict[str, str]] | None = None,
) -> list[str]:
    errors: list[str] = []
    canonical_rows = canonical_rows_override or {}
    canonical_statuses = ROW_STATUS
    if check_wording:
        try:
            canonical_rows = discover_product_rows(root)
            canonical_statuses = {name: row["status"] for name, row in canonical_rows.items()}
        except RuntimeError as error:
            errors.append(str(error))
        if canonical_statuses != ROW_STATUS:
            errors.append(
                "GH969 PRODUCT canonical statuses contradict discovery metadata: "
                f"product={canonical_statuses} code={ROW_STATUS}"
            )
    records = manifest.get("records")
    published_raw = manifest.get("published_surfaces")
    if manifest.get("schema_version") != 2 or not isinstance(records, list) or not isinstance(published_raw, dict):
        return ["surface manifest must have schema_version=2, published_surfaces, and a records array"]
    if not isinstance(manifest.get("published_release"), str) or not str(manifest.get("published_release", "")).strip():
        errors.append("surface manifest requires a non-empty published_release")
    published: dict[str, set[str]] = {}
    for kind in DISCOVERED_KINDS:
        entries = published_raw.get(kind)
        if not _string_list(entries):
            errors.append(f"published_surfaces.{kind} must be a non-empty string array")
            entries = []
        published[kind] = set(entries)
    ids: Counter[str] = Counter()
    rows: set[str] = set()
    recovery_guards: dict[str, list[dict[str, object]]] = defaultdict(list)
    caller_guards: dict[str, list[dict[str, object]]] = defaultdict(list)
    default_guards: dict[str, list[dict[str, object]]] = defaultdict(list)
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
            if canonical := canonical_rows.get(row_name):
                fields = {"canonical_entry": "entry", "owner": "owner", "default_state": "real_caller_default", "evidence": "evidence", "compatibility": "compatibility", "next_decision": "next_decision"}
                for manifest_field, product_field in fields.items():
                    if record.get(manifest_field) != canonical[product_field]:
                        errors.append(f"{label}: {manifest_field} contradicts canonical PRODUCT row {row_name}")
        if not isinstance(record.get("owner"), str) or not str(record.get("owner", "")).strip():
            errors.append(f"{label}: owner is required; assign the responsible source/spec owner")
        points = record.get("public_entry_points")
        if not _string_list(points):
            errors.append(f"{label}: public_entry_points must be a non-empty string array")
            points = []
        expected_status = canonical_statuses.get(str(row_name))
        if kind in DISCOVERED_KINDS and expected_status == "production" and len(points) == 1 and points[0] not in published.get(str(kind), set()):
            expected_status = "staged"
        if expected_status is not None and status != expected_status:
            errors.append(f"{label}: status {status!r} contradicts canonical/published status {expected_status!r}")
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
        caller_guard = record.get("caller_guard")
        if isinstance(row_name, str) and isinstance(caller_guard, dict):
            caller_guards[row_name].append(record)
        default_guard = record.get("default_guard")
        if isinstance(row_name, str) and isinstance(default_guard, dict):
            default_guards[row_name].append(record)
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
                try:
                    expected_categories = offline_categories(root, roots)
                except RuntimeError as error:
                    errors.append(f"{label}: {error}")
                    expected_categories = {name: [] for name in ("executables", "schemas", "fixtures", "data", "documents")}
                for category, expected in expected_categories.items():
                    declared = artifacts.get(category)
                    if not _string_list(declared) or set(declared) != set(expected):
                        actual = set(declared) if _string_list(declared) else set()
                        errors.append(
                            f"{label}: offline {category} inventory drift: "
                            f"missing={sorted(set(expected)-actual)} stale={sorted(actual-set(expected))}"
                        )
                command = artifacts.get("checked_command")
                executables = set(expected_categories["executables"])
                if not isinstance(command, str) or len(command.split()) < 2 or command.split()[1] not in executables:
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
        missing_published = sorted(published.get(kind, set()) - actual_entries)
        if missing_published:
            errors.append(f"published {kind} surfaces disappeared: {missing_published}")
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

    for row_name, symbols in EXPERIMENTAL_CALLER_SYMBOLS.items():
        if row_name not in rows:
            continue
        guards = caller_guards.get(row_name, [])
        if len(guards) != 1:
            errors.append(f"experimental row {row_name!r} requires exactly one implementation caller_guard")
            continue
        actual_guard = guards[0].get("caller_guard")
        expected_guard = build_caller_guard(root, symbols)
        if actual_guard != expected_guard:
            errors.append(
                f"experimental row {row_name!r} implementation callers changed; review default-path status and regenerate the manifest"
            )

    for row_name, mode in PRODUCTION_DEFAULT_GUARDS.items():
        if row_name not in rows:
            continue
        guards = default_guards.get(row_name, [])
        if len(guards) != 1 or guards[0].get("default_guard") != build_default_guard(root, mode):
            errors.append(f"production row {row_name!r} default implementation evidence changed")

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
            "## Canonical Surface Inventory\n\n| Inventory row | Entry | Owner | Status | Caller | Evidence | Compatibility | Decision |\n"
            "|---|---|---|---|---|---|---|---|\n| `fixture-row` | fixture | fixture | `production` | caller | evidence | compatible | continuous |\n"
            "\n## Decision Gates\n",
            encoding="utf-8",
        )
        (root / "Cargo.toml").write_text('[features]\ndefault = ["eval"]\neval = []\n', encoding="utf-8")
        (root / "src/mcp/server/tool_contracts.rs").write_text(
            'struct ToolContract; const CONTRACTS: [ToolContract; 1] = [json_object("search", "Search", true, false, true, true, Schema::Search)\n];\n', encoding="utf-8"
        )
        (root / "src/mcp/types.rs").write_text(
            "struct SearchParams { pub query: Option<String>, pub task_intent: Option<String> }\n",
            encoding="utf-8",
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
        (root / "src/platform.rs").write_text(
            "pub fn windows_api() {}\n", encoding="utf-8"
        )
        (root / "src/lib.rs").write_text("#[cfg(windows)] pub mod platform;\n", encoding="utf-8")
        doc_root = root / "doc/remem"
        (doc_root / "api").mkdir(parents=True)
        (doc_root / "index.html").write_text(
            '<a class="mod" href="api/index.html">api</a>', encoding="utf-8"
        )
        (doc_root / "api/index.html").write_text("module", encoding="utf-8")
        (doc_root / "all.html").write_text(
            '<ul class="all-items"><li><a href="api/struct.RouterInfo.html">api::RouterInfo</a></li>'
            '<li><a href="api/trait.Health.html">api::Health</a></li></ul>',
            encoding="utf-8",
        )
        (doc_root / "api/struct.RouterInfo.html").write_text(
            '<pre class="rust item-decl"><code>pub struct RouterInfo { pub status: bool }</code></pre>'
            '<span id="structfield.status"></span><div id="implementations-list">'
            '<section id="method.health" class="method"><h4 class="code-header">pub fn health(&amp;self)</h4></section></div><h2 id="trait-implementations"></h2>'
            '<section id="method.clone" class="method trait-impl"></section>',
            encoding="utf-8",
        )
        (doc_root / "api/trait.Health.html").write_text(
            '<pre class="rust item-decl"><code>pub trait Health</code></pre>'
            '<section id="method.defaulted"><h4 class="code-header">fn defaulted(&amp;self)</h4></section>'
            '<section id="tymethod.required"><h4 class="code-header">fn required(&amp;self)</h4></section>'
            '<h2 id="implementations"></h2>', encoding="utf-8"
        )
        discovered = discover_all(root, doc_root=doc_root)
        router_doc = doc_root / "api/struct.RouterInfo.html"
        original_router_doc = router_doc.read_text(encoding="utf-8")
        router_doc.write_text(original_router_doc.replace("status: bool", "status: String"), encoding="utf-8")
        changed_rust = discover_all(root, doc_root=doc_root)["rust_export"]
        router_doc.write_text(original_router_doc, encoding="utf-8")
        if changed_rust == discovered["rust_export"]:
            sys.stderr.write("Rust public signature change did not alter the compatibility fingerprint\n")
            return 1
        expected = {
            "remem context", "remem ctx", "remem eval", "remem admin",
            "remem admin backup", "remem admin save", "remem help", "remem admin help",
        }
        expected_rust = {
            "remem::api", "remem::api::RouterInfo", "remem::api::RouterInfo::status",
            "remem::api::RouterInfo::health", "remem::api::Health",
            "remem::api::Health::defaulted", "remem::api::Health::required",
        }
        expected_rest = {
            "GET /health", "HEAD /health", "POST /health", "POST /api/v1/admin/check",
        }
        if (
            discovered["cli_command"] != expected
            or discovered["rest_route"] != expected_rest
            or not expected_rust.issubset({entry.split("@sha256=", 1)[0] for entry in discovered["rust_export"]})
            or any(entry.startswith("remem::api::RouterInfo::clone") for entry in discovered["rust_export"])
            or not any("src/platform.rs::windows::platform::fn:windows_api" in entry for entry in discovered["rust_target_export"])
        ):
            print(f"surface discovery self-test failed: {discovered}", file=sys.stderr)
            return 1
        duplicate_enum = root / "src/cli/duplicate.rs"
        duplicate_enum.write_text(
            "#[cfg(feature = \"eval\")] #[derive(Subcommand)] enum FeatureAction { On }\n"
            "#[cfg(not(feature = \"eval\"))] #[derive(Subcommand)] enum FeatureAction { Off }\n",
            encoding="utf-8",
        )
        discover_cli_commands(root)
        duplicate_enum.write_text(
            "#[derive(Subcommand)] enum AdminAction { Restore }\n", encoding="utf-8"
        )
        try:
            discover_cli_commands(root)
        except RuntimeError as error:
            if "ambiguous Clap Subcommand enum basename" not in str(error):
                raise
        else:
            sys.stderr.write("duplicate Clap enum basename did not fail closed\n")
            return 1
        duplicate_enum.unlink()
        (root / "Cargo.toml").write_text('[features]\ndefault = []\neval = []\n', encoding="utf-8")
        if "remem eval" in discover_cli_commands(root):
            sys.stderr.write("default-feature CLI discovery retained disabled eval command\n")
            return 1
        (root / "Cargo.toml").write_text(
            '[features]\ndefault = ["full"]\nfull = ["eval"]\neval = []\n', encoding="utf-8"
        )
        if "remem eval" not in discover_cli_commands(root):
            sys.stderr.write("transitive default feature did not enable eval command\n")
            return 1
        (root / "Cargo.toml").write_text('[features]\ndefault = ["eval"]\neval = []\n', encoding="utf-8")
        if discover_product_rows(root)["fixture-row"]["compatibility"] != "compatible":
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
        (root / "src/runtime.rs").write_text(
            "fn route() { crate::retrieval_router::plan(); }", encoding="utf-8"
        )
        caller_guard = build_caller_guard(root, ("crate::retrieval_router",))
        (root / "src/runtime.rs").write_text(
            "fn route_changed() { crate::retrieval_router::plan(); }", encoding="utf-8"
        )
        if caller_guard == build_caller_guard(root, ("crate::retrieval_router",)):
            sys.stderr.write("experimental caller fingerprint ignored an implementation change\n")
            return 1
        (root / "src/context").mkdir()
        (root / "src/context/render_bundle.rs").write_text(
            'match mode { "" | "bundle" => Ok(ContextBundleRenderMode::Bundle), '
            'Err(std::env::VarError::NotPresent) => Ok(ContextBundleRenderMode::Bundle) }',
            encoding="utf-8",
        )
        weights = root / "src/retrieval/search/memory/weights.rs"
        weights.parent.mkdir(parents=True)
        weights.write_text("const GRAPH_WEIGHT: f64 = 0.75;\n", encoding="utf-8")
        if build_default_guard(root, "positive_graph_weight")["value"] != 0.75:
            sys.stderr.write("production graph default evidence was not resolved\n")
            return 1
        weights.write_text("const GRAPH_WEIGHT: f64 = 0.0;\n", encoding="utf-8")
        try:
            build_default_guard(root, "positive_graph_weight")
        except RuntimeError:
            pass
        else:
            sys.stderr.write("zero graph default did not fail production evidence\n")
            return 1
        weights.write_text("const GRAPH_WEIGHT: f64 = 0.75;\n", encoding="utf-8")
        offline_roots = ["eval/cross-host", "docs/specs/GH935"]
        (root / "eval/cross-host/scripts").mkdir(parents=True)
        (root / "eval/cross-host/scripts/check.py").write_text("print('ok')\n", encoding="utf-8")
        (root / "docs/specs/GH935").mkdir(parents=True)
        (root / "docs/specs/GH935/PRODUCT.md").write_text("fixture", encoding="utf-8")
        (root / "docs/specs/GH935/TECH.md").write_text("fixture", encoding="utf-8")
        records = [
            lifecycle_record(f"{kind}:{entry}", kind, entry, lifecycle_row(kind, entry))
            for kind, entries in discovered.items() for entry in entries
        ]
        records.append(lifecycle_record(
            "offline:fixture", "offline_harness", offline_roots[0], "cross-host-harness",
            artifacts={
                "roots": offline_roots,
                **offline_categories(root, offline_roots),
                "checked_command": "python3 eval/cross-host/scripts/check.py",
            },
        ))
        for row_name, symbols in EXPERIMENTAL_CALLER_SYMBOLS.items():
            guarded = next(
                (record for record in records if record["inventory_row"] == row_name), None
            )
            if guarded is not None:
                guarded["caller_guard"] = build_caller_guard(root, symbols)
        for item in records:
            for spec in item["spec_refs"]:
                spec_path = root / str(spec)
                spec_path.parent.mkdir(parents=True, exist_ok=True)
                if not spec_path.exists():
                    spec_path.write_text("fixture", encoding="utf-8")
        manifest: dict[str, object] = {
            "schema_version": 2,
            "published_release": "v1.0.0",
            "published_surfaces": {kind: sorted(entries) for kind, entries in discovered.items()},
            "records": records,
        }
        fixture_rows: dict[str, dict[str, str]] = {}
        for record in records:
            row_name = str(record["inventory_row"])
            fixture_rows.setdefault(row_name, {
                "entry": str(record["canonical_entry"]), "owner": str(record["owner"]),
                "status": ROW_STATUS[row_name], "real_caller_default": str(record["default_state"]),
                "evidence": str(record["evidence"]), "compatibility": str(record["compatibility"]),
                "next_decision": str(record["next_decision"]),
            })
        positive_errors = check_lifecycle_manifest(
            root, manifest, discovered, today=date(2026, 8, 24), required_rows=set(), canonical_rows_override=fixture_rows
        )
        if positive_errors:
            print(f"positive lifecycle manifest self-test failed: {positive_errors}", file=sys.stderr)
            return 1

        def proves(mutator: object, needle: str) -> bool:
            candidate = copy.deepcopy(manifest)
            assert callable(mutator)
            mutator(candidate)
            return any(needle in error for error in check_lifecycle_manifest(root, candidate, discovered, today=date(2026, 8, 24), required_rows=set(), canonical_rows_override=fixture_rows))

        def miscategorize_offline(value: dict[str, object]) -> None:
            offline = next(record for record in value["records"] if record["id"] == "offline:fixture")
            artifacts = offline["artifacts"]
            script = artifacts["executables"].pop()
            artifacts["documents"].append(script)

        def remove_from_published(value: dict[str, object]) -> None:
            record = next(item for item in value["records"] if item["surface_kind"] in DISCOVERED_KINDS and item["status"] == "production")
            value["published_surfaces"][record["surface_kind"]].remove(record["public_entry_points"][0])

        cases = [
            (lambda value: value["records"][0].update(status="mystery"), "unknown lifecycle status"),
            (lambda value: value["records"][0].update(owner=""), "owner is required"),
            (lambda value: value["records"][0].update(compatibility="changed"), "compatibility contradicts canonical PRODUCT"),
            (lambda value: value["records"].append(copy.deepcopy(value["records"][0])), "multiply classified"),
            (lambda value: value["records"].pop(0), "unclassified"),
            (lambda value: value["records"][0]["public_entry_points"].__setitem__(0, "stale"), "stale"),
            (lambda value: value["records"][0].update(status="production", real_callers=["none"]), "no real production caller"),
            (lambda value: value["records"][0].update(status="experimental", inventory_row="mcp-context-bundle", decision_due="2026-01-01"), "overdue"),
            (lambda value: value["records"][0].update(status="experimental", inventory_row="mcp-context-bundle", decision_due="2026-11-30", default_state="default path"), "without a separately classified"),
            (lambda value: value["records"][0].update(status="recovery-only", inventory_row="legacy-pending", normal_writers=["new work"]), "new-work writers are forbidden"),
            (lambda value: value["records"][0].update(spec_refs=["missing.md"]), "does not resolve"),
            (miscategorize_offline, "offline executables inventory drift"),
            (remove_from_published, "canonical/published status 'staged'"),
        ]
        failed = [needle for mutator, needle in cases if not proves(mutator, needle)]
        if failed:
            print(f"negative lifecycle self-tests failed: {failed}", file=sys.stderr)
            return 1
        unsupported = root / "eval/cross-host/scripts/check.sh"
        unsupported.write_text("#!/bin/sh\n", encoding="utf-8")
        try:
            offline_categories(root, offline_roots)
        except RuntimeError as error:
            if "unsupported offline script artifact" not in str(error):
                raise
        else:
            sys.stderr.write("uncategorized offline artifact did not fail closed\n")
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
