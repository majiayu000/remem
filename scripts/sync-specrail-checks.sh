#!/usr/bin/env bash
# Sync vendored SpecRail gate scripts and schemas from an upstream specrail checkout.
#
# Usage:
#   scripts/sync-specrail-checks.sh /path/to/specrail   # copy files, rewrite lock
#   scripts/sync-specrail-checks.sh --verify            # check vendored files against lock
#
# checks/check_workflow.py is intentionally excluded: it is repo-specific
# (REQUIRED_FILES lists remem's own adoption surface) and must be maintained
# by hand.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
LOCK_FILE="$REPO_ROOT/checks/specrail-sync.lock.json"

SYNCED_FILES=(
  "checks/duplicate_work_gate.py"
  "checks/github_duplicate_evidence.py"
  "checks/github_issue_evidence.py"
  "checks/github_pr_evidence.py"
  "checks/pr_gate.py"
  "checks/review_json_gate.py"
  "checks/route_gate.py"
  "checks/runtime_ledger_gate.py"
  "checks/specrail_lib.py"
  "schemas/duplicate_work_evidence.schema.json"
  "schemas/pr_review_gate.schema.json"
  "schemas/review_result.schema.json"
  "schemas/runtime_checkpoint.schema.json"
)

write_lock() {
  local upstream_sha="$1"
  python3 - "$LOCK_FILE" "$upstream_sha" "${SYNCED_FILES[@]}" <<'PY'
import hashlib
import json
import os
import sys

lock_path, upstream_sha, *files = sys.argv[1:]
repo_root = os.path.dirname(os.path.dirname(os.path.abspath(lock_path)))
entries = []
for rel in files:
    path = os.path.join(repo_root, rel)
    digest = hashlib.sha256(open(path, "rb").read()).hexdigest()
    entries.append({"path": rel, "sha256": digest})
lock = {
    "upstream_repo": "https://github.com/majiayu000/specrail",
    "upstream_sha": upstream_sha,
    "excluded": ["checks/check_workflow.py"],
    "files": entries,
}
with open(lock_path, "w") as fh:
    json.dump(lock, fh, indent=2)
    fh.write("\n")
print(f"lock written: {lock_path} @ upstream {upstream_sha}")
PY
}

verify_lock() {
  python3 - "$LOCK_FILE" <<'PY'
import hashlib
import json
import os
import sys

lock_path = sys.argv[1]
repo_root = os.path.dirname(os.path.dirname(os.path.abspath(lock_path)))
with open(lock_path) as fh:
    lock = json.load(fh)
failed = False
for entry in lock["files"]:
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
print(f"ok: {len(lock['files'])} files match lock (upstream {lock['upstream_sha']})")
PY
}

if [[ "${1:-}" == "--verify" ]]; then
  verify_lock
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
