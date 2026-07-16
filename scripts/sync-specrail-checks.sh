#!/usr/bin/env bash
# Sync vendored SpecRail gate scripts and schemas from an upstream specrail checkout.
#
# Usage:
#   scripts/sync-specrail-checks.sh /path/to/specrail   # copy files, rewrite lock
#   scripts/sync-specrail-checks.sh --verify            # check vendored files against lock
#
# Repo-specific Python checks are local-owned and explicitly excluded from
# upstream sync. Every tracked checks/*.py file and repo-local import must be
# classified as either upstream-managed or local-owned.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
LOCK_FILE="$REPO_ROOT/checks/specrail-sync.lock.json"

SYNCED_FILES=(
  "checks/duplicate_work_gate.py"
  "checks/github_duplicate_evidence.py"
  "checks/github_evidence_common.py"
  "checks/github_issue_evidence.py"
  "checks/github_issue_reference.py"
  "checks/github_pr_evidence.py"
  "checks/pr_gate.py"
  "checks/review_json_gate.py"
  "checks/route_gate.py"
  "checks/runtime_gate_rules.py"
  "checks/runtime_ledger_gate.py"
  "checks/specrail_lib.py"
  "schemas/duplicate_work_evidence.schema.json"
  "schemas/pr_review_gate.schema.json"
  "schemas/review_result.schema.json"
  "schemas/runtime_checkpoint.schema.json"
)

LOCAL_OWNED_FILES=(
  "checks/check_workflow.py"
  "checks/schema_contract.py"
)

write_lock() {
  local upstream_sha="$1"
  python3 - "$LOCK_FILE" "$upstream_sha" "${SYNCED_FILES[@]}" -- "${LOCAL_OWNED_FILES[@]}" <<'PY'
import hashlib
import json
import os
import sys

lock_path, upstream_sha, *classified = sys.argv[1:]
separator = classified.index("--")
files = classified[:separator]
excluded = classified[separator + 1:]
repo_root = os.path.dirname(os.path.dirname(os.path.abspath(lock_path)))
entries = []
for rel in files:
    path = os.path.join(repo_root, rel)
    digest = hashlib.sha256(open(path, "rb").read()).hexdigest()
    entries.append({"path": rel, "sha256": digest})
lock = {
    "upstream_repo": "https://github.com/majiayu000/specrail",
    "upstream_sha": upstream_sha,
    "excluded": excluded,
    "files": entries,
}
with open(lock_path, "w") as fh:
    json.dump(lock, fh, indent=2)
    fh.write("\n")
print(f"lock written: {lock_path} @ upstream {upstream_sha}")
PY
}

verify_lock() {
  python3 - "$LOCK_FILE" "${SYNCED_FILES[@]}" -- "${LOCAL_OWNED_FILES[@]}" <<'PY'
import hashlib
import json
import os
import sys

lock_path, *classified = sys.argv[1:]
separator = classified.index("--")
expected_files = classified[:separator]
expected_excluded = classified[separator + 1:]
repo_root = os.path.dirname(os.path.dirname(os.path.abspath(lock_path)))
with open(lock_path) as fh:
    lock = json.load(fh)
failed = False
entries = lock.get("files")
if not isinstance(entries, list):
    print("INVALID: lock files must be a list")
    sys.exit(1)
lock_files = [entry.get("path") for entry in entries if isinstance(entry, dict)]
if len(lock_files) != len(entries) or lock_files != expected_files:
    print("INVALID: sync managed file list does not match lock")
    print(f"script: {expected_files}")
    print(f"lock:   {lock_files}")
    failed = True
excluded = lock.get("excluded")
if excluded != expected_excluded:
    print("INVALID: local-owned excluded file list does not match lock")
    print(f"script: {expected_excluded}")
    print(f"lock:   {excluded}")
    failed = True
for entry in entries:
    if not isinstance(entry, dict):
        continue
    path = os.path.join(repo_root, entry["path"])
    if not os.path.exists(path):
        print(f"MISSING: {entry['path']}")
        failed = True
        continue
    digest = hashlib.sha256(open(path, "rb").read()).hexdigest()
    if digest != entry["sha256"]:
        print(f"DRIFT: {entry['path']}")
        failed = True
if failed:
    print(f"vendored SpecRail files drifted from lock (upstream {lock['upstream_sha']}); "
          "re-run scripts/sync-specrail-checks.sh <specrail-repo> or restore the files")
    sys.exit(1)
print(f"ok: {len(lock['files'])} upstream-managed files match lock "
      f"(upstream {lock['upstream_sha']})")
print(f"ok: {len(expected_excluded)} local-owned files explicitly excluded from upstream sync")
PY
}

verify_python_imports() {
  python3 - "$LOCK_FILE" "$REPO_ROOT" <<'PY'
import ast
import importlib
import json
import subprocess
import sys
from pathlib import Path

lock_path = Path(sys.argv[1])
repo_root = Path(sys.argv[2])
with lock_path.open(encoding="utf-8") as fh:
    lock = json.load(fh)

checks_dir = repo_root / "checks"
managed = {Path(entry["path"]) for entry in lock["files"]}
excluded = {Path(path) for path in lock["excluded"]}
managed_python = {path for path in managed if path.parent == Path("checks") and path.suffix == ".py"}
excluded_python = {path for path in excluded if path.parent == Path("checks") and path.suffix == ".py"}
classified_python = managed_python | excluded_python

tracked = subprocess.run(
    ["git", "-C", str(repo_root), "ls-files", "--", "checks"],
    capture_output=True,
    text=True,
    check=False,
)
if tracked.returncode != 0:
    print(f"TRACKING FAILED: {tracked.stderr.strip()}", file=sys.stderr)
    raise SystemExit(1)
tracked_python = {
    Path(line)
    for line in tracked.stdout.splitlines()
    if line and Path(line).suffix == ".py"
}
unclassified = sorted(tracked_python - classified_python)
untracked = sorted(classified_python - tracked_python)
if unclassified or untracked:
    for path in unclassified:
        print(f"UNCLASSIFIED TRACKED PYTHON: {path}", file=sys.stderr)
    for path in untracked:
        print(f"CLASSIFIED PYTHON IS NOT TRACKED: {path}", file=sys.stderr)
    raise SystemExit(1)

repo_root_resolved = repo_root.resolve()
checks_dir_resolved = checks_dir.resolve()


def existing_local_paths(module):
    parts = module.split(".")
    if parts and parts[0] == "checks":
        parts = parts[1:]
    if not parts or any(not part or part in {".", ".."} for part in parts):
        return []
    base = checks_dir.joinpath(*parts)
    candidates = (base.with_suffix(".py"), base / "__init__.py")
    resolved_paths = []
    for candidate in candidates:
        if not candidate.is_file():
            continue
        resolved = candidate.resolve()
        try:
            resolved.relative_to(checks_dir_resolved)
            relative = resolved.relative_to(repo_root_resolved)
        except ValueError:
            print(f"LOCAL IMPORT PATH ESCAPE: {candidate}", file=sys.stderr)
            raise SystemExit(1)
        resolved_paths.append(relative)
    return resolved_paths


def require_classified_import(source, module):
    for candidate in existing_local_paths(module):
        if candidate not in classified_python:
            print(
                f"UNCLASSIFIED LOCAL IMPORT: {source} imports {candidate}",
                file=sys.stderr,
            )
            raise SystemExit(1)


for relative_path in sorted(tracked_python):
    source_path = repo_root / relative_path
    try:
        tree = ast.parse(source_path.read_text(encoding="utf-8"), filename=str(relative_path))
    except (OSError, SyntaxError) as exc:
        print(f"AST FAILED: {relative_path}: {type(exc).__name__}: {exc}", file=sys.stderr)
        raise SystemExit(1) from exc
    for node in ast.walk(tree):
        if isinstance(node, ast.Import):
            for alias in node.names:
                require_classified_import(relative_path, alias.name)
        elif isinstance(node, ast.ImportFrom):
            if node.level:
                reason = (
                    "LOCAL IMPORT PATH ESCAPE"
                    if node.level > 1
                    else "UNSUPPORTED RELATIVE LOCAL IMPORT"
                )
                print(
                    f"{reason}: {relative_path}: checks/ is a flat non-package layout",
                    file=sys.stderr,
                )
                raise SystemExit(1)
            module = node.module or ""
            require_classified_import(relative_path, module)
            for alias in node.names:
                if alias.name != "*":
                    qualified = f"{module}.{alias.name}" if module else alias.name
                    require_classified_import(relative_path, qualified)

sys.path.insert(0, str(checks_dir))
for relative_path in sorted(classified_python):
    try:
        importlib.import_module(relative_path.stem)
    except BaseException as exc:
        print(
            f"IMPORT FAILED: {relative_path}: {type(exc).__name__}: {exc}",
            file=sys.stderr,
        )
        raise SystemExit(1) from exc

print(f"ok: {len(managed_python)} upstream-managed Python files classified")
print(f"ok: {len(excluded_python)} local-owned excluded Python files classified")
print("ok: classified SpecRail Python import closure")
PY
}

verify_workflow() {
  python3 "$REPO_ROOT/checks/check_workflow.py" --repo "$REPO_ROOT"
}

if [[ "${1:-}" == "--verify" ]]; then
  verify_lock
  verify_python_imports
  verify_workflow
  exit 0
fi

UPSTREAM="${1:?usage: sync-specrail-checks.sh /path/to/specrail | --verify}"
if [[ ! -d "$UPSTREAM/checks" ]]; then
  echo "error: $UPSTREAM does not look like a specrail checkout (no checks/)" >&2
  exit 1
fi

upstream_sha="$(git -C "$UPSTREAM" rev-parse HEAD)"
for rel in "${SYNCED_FILES[@]}"; do
  cp "$UPSTREAM/$rel" "$REPO_ROOT/$rel"
done
write_lock "$upstream_sha"
verify_lock
verify_python_imports
verify_workflow
