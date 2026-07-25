#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
if [[ -n "${REMEM_API_SMOKE_PORT:-}" ]]; then
  PORT="$REMEM_API_SMOKE_PORT"
else
  PORT="$(
    python3 - <<'PY'
import socket

with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
    sock.bind(("127.0.0.1", 0))
    print(sock.getsockname()[1])
PY
  )"
fi
BASE_URL="http://127.0.0.1:${PORT}"
TMP_BASE="$(mktemp -d "${TMPDIR:-/tmp}/remem-api-smoke.XXXXXX")"
SERVER_PID=""

cleanup() {
  if [[ -n "$SERVER_PID" ]] && kill -0 "$SERVER_PID" 2>/dev/null; then
    kill "$SERVER_PID" 2>/dev/null || true
    wait "$SERVER_PID" 2>/dev/null || true
  fi
  rm -rf "$TMP_BASE"
}
trap cleanup EXIT

if [[ -z "${REMEM_BIN:-}" ]]; then
  REMEM_BIN="$(
    cargo build --bin remem --message-format=json 2>"$TMP_BASE/cargo-build.log" |
      python3 -c '
import json
import sys

executable = ""
for line in sys.stdin:
    try:
        message = json.loads(line)
    except json.JSONDecodeError:
        continue
    if (
        message.get("reason") == "compiler-artifact"
        and message.get("target", {}).get("name") == "remem"
        and message.get("executable")
    ):
        executable = message["executable"]

if executable:
    print(executable)
'
  )"
  if [[ -z "$REMEM_BIN" || ! -x "$REMEM_BIN" ]]; then
    echo "native web API smoke failed: cargo did not report an executable remem binary" >&2
    sed -n '1,120p' "$TMP_BASE/cargo-build.log" >&2 || true
    exit 1
  fi
elif [[ ! -x "$REMEM_BIN" ]]; then
  echo "native web API smoke failed: REMEM_BIN is not executable: $REMEM_BIN" >&2
  exit 1
fi

export HOME="$TMP_BASE/home"
export REMEM_DATA_DIR="$TMP_BASE/remem-data"
export REMEM_ALLOW_PLAINTEXT_DB=1
DB_PATH="$REMEM_DATA_DIR/remem.db"
mkdir -p "$HOME" "$REMEM_DATA_DIR"

"$REMEM_BIN" api --port "$PORT" >"$TMP_BASE/remem-api.log" 2>&1 &
SERVER_PID=$!
TOKEN_FILE="$REMEM_DATA_DIR/.api-token"
TOKEN=""

fail() {
  echo "native web API smoke failed: $*" >&2
  echo "--- remem api log ---" >&2
  sed -n '1,120p' "$TMP_BASE/remem-api.log" >&2 || true
  exit 1
}

validate_json() {
  local name="$1"
  local shape="$2"
  local file="$3"
  python3 - "$name" "$shape" "$file" <<'PY'
import json
import sys

name, shape, path = sys.argv[1], sys.argv[2], sys.argv[3]
with open(path, "r", encoding="utf-8") as handle:
    payload = json.load(handle)

def require(condition, message):
    if not condition:
        raise SystemExit(f"{name}: {message}")

def require_keys(obj, keys):
    require(isinstance(obj, dict), "expected JSON object")
    for key in keys:
        require(key in obj, f"missing key {key!r}")

if shape == "health":
    require_keys(payload, ["ok", "version", "api_version", "schema_version"])
    require(payload["ok"] is True, "ok should be true")
elif shape == "status":
    require_keys(payload, ["version", "cache"])
    require_keys(payload["cache"], ["hit", "stale", "generated_at_epoch", "ttl_secs"])
elif shape == "capabilities":
    require_keys(payload, ["version", "schema_version", "api_version", "features", "endpoints"])
    require(payload["features"].get("health") is True, "health feature should be true")
    require(payload["features"].get("graph") is True, "graph feature should be true")
    require(payload["features"].get("candidate_review") is True, "candidate_review feature should be true")
    require(payload["features"].get("user_recall") is True, "user_recall feature should be true")
    require(payload["endpoints"].get("user_recall") == "/api/v1/user/recall", "user_recall endpoint mismatch")
    expected_features = {
        "candidate_detail": True,
        "candidate_evidence": True,
        "candidate_review_safe": True,
        "observations": True,
        "sessions": True,
        "workstreams": True,
        "events": True,
        "tasks": True,
        "memory_archive": True,
        "memory_restore": True,
        "memory_delete": False,
    }
    for key, expected in expected_features.items():
        require(payload["features"].get(key) is expected, f"feature {key!r} mismatch")
    expected_endpoints = {
        "candidate_detail": "/api/v1/candidates/{id}",
        "candidate_evidence": "/api/v1/candidates/{id}",
        "candidate_review_safe_approve": "/api/v1/candidates/{id}/review/approve",
        "candidate_review_safe_reject": "/api/v1/candidates/{id}/review/reject",
        "candidate_review_safe_edit": "/api/v1/candidates/{id}/review/edit",
        "observations_list": "/api/v1/observations",
        "observations_detail": "/api/v1/observations/{id}",
        "sessions_list": "/api/v1/sessions",
        "sessions_detail": "/api/v1/sessions/{id}",
        "workstreams_list": "/api/v1/workstreams",
        "workstreams_detail": "/api/v1/workstreams/{id}",
        "events_list": "/api/v1/events",
        "events_detail": "/api/v1/events/{id}",
        "tasks_list": "/api/v1/tasks",
        "tasks_detail": "/api/v1/tasks/{id}",
        "memory_archive": "/api/v1/memories/{id}/archive",
        "memory_restore": "/api/v1/memories/{id}/restore",
    }
    for key, expected in expected_endpoints.items():
        require(payload["endpoints"].get(key) == expected, f"endpoint {key!r} mismatch")
    require("memory_delete" not in payload["endpoints"], "memory_delete endpoint must be absent")
elif shape == "list":
    require_keys(payload, ["data", "meta"])
    require(isinstance(payload["data"], list), "data should be an array")
    require_keys(payload["meta"], ["count", "total", "limit", "offset", "has_more", "next_offset"])
elif shape == "search":
    require_keys(payload, ["data", "meta"])
    require(isinstance(payload["data"], list), "data should be an array")
    require_keys(payload["meta"], ["count", "has_more", "limit", "offset"])
elif shape == "legacy_raw_search":
    require_keys(payload, ["data", "meta", "raw_hits"])
    require(isinstance(payload["raw_hits"], list), "raw_hits should be an array")
    require(
        any("legacy raw smoke sentinel" in str(item.get("preview", "")) for item in payload["raw_hits"]),
        "legacy raw-hit preview contract missing sentinel",
    )
elif shape == "resource_list":
    require_keys(payload, ["data", "page_size", "next_cursor"])
    require(isinstance(payload["data"], list), "data should be an array")
    require(isinstance(payload["page_size"], int), "page_size should be an integer")
elif shape == "resource_detail":
    require_keys(payload, ["data"])
    require(isinstance(payload["data"], dict), "data should be an object")
elif shape == "candidate_detail":
    require_keys(payload, ["data", "evidence", "decision"])
    require(isinstance(payload["evidence"], list), "evidence should be an array")
    require_keys(payload["decision"], ["can_review", "blocked_reasons"])
elif shape == "memory_detail":
    require_keys(payload, ["id", "version", "status", "entities", "edges"])
    require(isinstance(payload["version"], int), "memory version should be an integer")
elif shape == "safe_mutation":
    require_keys(payload, [
        "response_schema_version",
        "operation_id",
        "audit_id",
        "action",
        "before_status",
        "after_status",
        "version",
        "occurred_at_epoch",
        "replayed",
    ])
    require(payload["response_schema_version"] == 1, "response schema should be 1")
    require(payload["replayed"] is False, "first mutation must not be a replay")
    require("idempotency_key" not in payload, "raw idempotency key leaked")
elif shape == "graph":
    require_keys(payload, ["nodes", "edges"])
    require(isinstance(payload["nodes"], list), "nodes should be an array")
    require(isinstance(payload["edges"], list), "edges should be an array")
elif shape == "stats":
    require_keys(payload, [
        "active_memories",
        "total_memories",
        "pending_candidates",
        "captured_events",
        "pending_extraction_tasks",
        "ai_calls",
        "ai_cost_usd",
        "ai_total_tokens",
        "type_distribution",
    ])
    require(isinstance(payload["type_distribution"], list), "type_distribution should be an array")
elif shape == "user_recall":
    require_keys(payload, ["query", "project", "empty", "context", "included", "dropped", "diagnostics"])
    require(isinstance(payload["included"], list), "included should be an array")
    require(isinstance(payload["dropped"], list), "dropped should be an array")
    require_keys(payload["diagnostics"], ["requested_limit", "budget_chars", "used_chars", "candidate_counts"])
elif shape == "error":
    require_keys(payload, ["error"])
    require_keys(payload["error"], ["code", "message"])
else:
    raise SystemExit(f"unknown shape {shape!r}")
PY
}

request_json() {
  local name="$1"
  local path="$2"
  local expected_code="$3"
  local shape="$4"
  local body="$TMP_BASE/${name}.json"
  local code

  code="$(curl -sS -o "$body" -w '%{http_code}' \
    -H "Authorization: Bearer ${TOKEN}" \
    "${BASE_URL}${path}" || true)"
  [[ "$code" == "$expected_code" ]] || fail "$name returned HTTP $code, expected $expected_code"
  validate_json "$name" "$shape" "$body"
  echo "ok $name"
}

request_json_post() {
  local name="$1"
  local path="$2"
  local expected_code="$3"
  local shape="$4"
  local payload="$5"
  local body="$TMP_BASE/${name}.json"
  local code

  code="$(curl -sS -o "$body" -w '%{http_code}' \
    -H "Authorization: Bearer ${TOKEN}" \
    -H "Content-Type: application/json" \
    -d "$payload" \
    "${BASE_URL}${path}" || true)"
  [[ "$code" == "$expected_code" ]] || fail "$name returned HTTP $code, expected $expected_code"
  validate_json "$name" "$shape" "$body"
  echo "ok $name"
}

request_json_no_auth() {
  local name="$1"
  local path="$2"
  local expected_code="$3"
  local shape="$4"
  local body="$TMP_BASE/${name}.json"
  local code

  code="$(curl -sS -o "$body" -w '%{http_code}' \
    "${BASE_URL}${path}" || true)"
  [[ "$code" == "$expected_code" ]] || fail "$name returned HTTP $code, expected $expected_code"
  validate_json "$name" "$shape" "$body"
  echo "ok $name"
}

request_code() {
  local name="$1"
  local method="$2"
  local path="$3"
  local expected_code="$4"
  local body="$TMP_BASE/${name}.txt"
  local code

  code="$(curl -sS -o "$body" -w '%{http_code}' \
    -X "$method" \
    -H "Authorization: Bearer ${TOKEN}" \
    "${BASE_URL}${path}" || true)"
  [[ "$code" == "$expected_code" ]] || fail "$name returned HTTP $code, expected $expected_code"
  echo "ok $name"
}

endpoint_path() {
  local key="$1"
  local resource_id="${2:-}"
  python3 - "$TMP_BASE/capabilities.json" "$key" "$resource_id" <<'PY'
import json
import sys

path, key, resource_id = sys.argv[1:]
with open(path, "r", encoding="utf-8") as handle:
    template = json.load(handle)["endpoints"][key]
if resource_id:
    template = template.replace("{id}", resource_id)
print(template)
PY
}

json_field() {
  local file="$1"
  local field="$2"
  python3 - "$file" "$field" <<'PY'
import json
import sys

with open(sys.argv[1], "r", encoding="utf-8") as handle:
    value = json.load(handle)[sys.argv[2]]
print(value)
PY
}

assert_no_token_leak() {
  [[ -n "$TOKEN" ]] || return 0
  if grep -Fq -- "$TOKEN" "$TMP_BASE/remem-api.log" "$TMP_BASE"/*.json 2>/dev/null; then
    fail "API token appeared in smoke output"
  fi
}

assert_no_idempotency_key_leak() {
  local key
  for key in \
    smoke-candidate-approve \
    smoke-candidate-reject \
    smoke-candidate-edit \
    smoke-memory-archive \
    smoke-memory-restore; do
    if grep -aFq -- "$key" "$TMP_BASE/remem-api.log" "$TMP_BASE"/*.json "$REMEM_DATA_DIR"/remem.db* 2>/dev/null; then
      fail "raw idempotency key appeared in DB, log, or response: $key"
    fi
  done
}

for _ in $(seq 1 80); do
  if ! kill -0 "$SERVER_PID" 2>/dev/null; then
    fail "server exited before becoming ready"
  fi
  if [[ -s "$TOKEN_FILE" ]]; then
    TOKEN="$(tr -d '\n\r' <"$TOKEN_FILE")"
    if [[ -n "$TOKEN" ]]; then
      body="$TMP_BASE/ready-health.json"
      code="$(curl -sS -o "$body" -w '%{http_code}' \
        -H "Authorization: Bearer ${TOKEN}" \
        "${BASE_URL}/api/v1/health" || true)"
      if [[ "$code" == "200" ]]; then
        validate_json "health" "health" "$body"
        break
      fi
    fi
  fi
  sleep 0.25
done

[[ -n "$TOKEN" ]] || fail "API token was not created"

schema_code="$(curl -sS -o "$TMP_BASE/schema-ready.json" -w '%{http_code}' \
  -H "Authorization: Bearer ${TOKEN}" \
  "${BASE_URL}/api/v1/memories?limit=1" || true)"
[[ "$schema_code" == "200" ]] || fail "schema initialization returned HTTP $schema_code"

python3 - "$DB_PATH" "$TMP_BASE/fixture.json" <<'PY'
import json
import sqlite3
import sys
import time

db_path, output_path = sys.argv[1:]
now = int(time.time())
conn = sqlite3.connect(db_path, timeout=10)
conn.execute("PRAGMA foreign_keys = ON")
with conn:
    conn.execute(
        "INSERT OR IGNORE INTO hosts(name, enabled, created_at_epoch) VALUES ('codex-cli', 1, ?)",
        (now,),
    )
    host_id = conn.execute("SELECT id FROM hosts WHERE name = 'codex-cli'").fetchone()[0]
    workspace_id = conn.execute(
        "INSERT INTO workspaces(root_path, created_at_epoch, updated_at_epoch) VALUES (?, ?, ?)",
        ("/tmp/remem-native-smoke", now, now),
    ).lastrowid
    project_id = conn.execute(
        "INSERT INTO projects(workspace_id, project_path, project_key, created_at_epoch, updated_at_epoch) "
        "VALUES (?, ?, ?, ?, ?)",
        (workspace_id, "/tmp/remem-native-smoke", "native-smoke", now, now),
    ).lastrowid
    session_id = conn.execute(
        "INSERT INTO sessions(host_id, workspace_id, project_id, session_id, started_at_epoch, "
        "last_seen_at_epoch, status) VALUES (?, ?, ?, ?, ?, ?, 'active')",
        (host_id, workspace_id, project_id, "native-smoke-session", now, now),
    ).lastrowid
    event_id = conn.execute(
        "INSERT INTO captured_events(host_id, workspace_id, project_id, session_row_id, session_id, "
        "event_id, event_type, role, tool_name, content_text, content_hash, token_estimate, "
        "retention_class, created_at_epoch, inserted_at_epoch) "
        "VALUES (?, ?, ?, ?, ?, ?, 'file_edit', 'tool', 'Edit', ?, ?, 4, 'inline', ?, ?)",
        (
            host_id,
            workspace_id,
            project_id,
            session_id,
            "native-smoke-session",
            "native-smoke-event",
            "legacy raw smoke sentinel",
            "native-smoke-content-hash",
            now,
            now,
        ),
    ).lastrowid
    conn.execute(
        "INSERT INTO raw_messages(session_id, project, role, content, content_hash, source, "
        "created_at_epoch, source_root) VALUES (?, ?, 'user', ?, ?, 'hook', ?, 'local')",
        (
            "native-smoke-raw-session",
            "native-smoke",
            "legacy raw smoke sentinel",
            "native-smoke-raw-hash",
            now,
        ),
    )
    task_id = conn.execute(
        "INSERT INTO extraction_tasks(task_kind, host_id, workspace_id, project_id, session_row_id, "
        "priority, status, idempotency_key, attempts, created_at_epoch, updated_at_epoch) "
        "VALUES ('memory_candidate', ?, ?, ?, ?, 20, 'pending', ?, 0, ?, ?)",
        (host_id, workspace_id, project_id, session_id, "native-smoke-task", now, now),
    ).lastrowid
    observation_id = conn.execute(
        "INSERT INTO observations(memory_session_id, project, type, title, narrative, "
        "created_at_epoch, status, project_id, session_row_id, observation_type, "
        "reference_time_epoch) VALUES (?, ?, 'discovery', ?, ?, ?, 'active', ?, ?, "
        "'discovery', ?)",
        (
            "native-smoke-session",
            "native-smoke",
            "Native smoke observation",
            "Safe derived observation",
            now,
            project_id,
            session_id,
            now,
        ),
    ).lastrowid
    workstream_id = conn.execute(
        "INSERT INTO workstreams(project, title, description, status, created_at_epoch, "
        "updated_at_epoch) VALUES (?, ?, ?, 'active', ?, ?)",
        ("native-smoke", "Native smoke workstream", "Safe progress", now, now),
    ).lastrowid
    candidate_ids = []
    for action in ("approve", "reject", "edit"):
        candidate_ids.append(
            conn.execute(
                "INSERT INTO memory_candidates(project_id, scope, memory_type, topic_key, text, "
                "evidence_event_ids, confidence, risk_class, review_status, created_at_epoch, "
                "updated_at_epoch) VALUES (?, 'project', 'decision', ?, ?, ?, 0.9, 'low', "
                "'pending_review', ?, ?)",
                (
                    project_id,
                    f"native-smoke-{action}",
                    f"Native smoke candidate for {action}",
                    json.dumps([event_id]),
                    now,
                    now,
                ),
            ).lastrowid
        )
    memory_id = conn.execute(
        "INSERT INTO memories(session_id, project, topic_key, title, content, memory_type, "
        "created_at_epoch, updated_at_epoch, status) VALUES (?, ?, ?, ?, ?, 'decision', ?, ?, "
        "'active')",
        (
            "native-smoke-session",
            "native-smoke",
            "native-smoke-memory",
            "Native smoke memory",
            "Recoverable native smoke memory",
            now,
            now,
        ),
    ).lastrowid

fixture = {
    "observation_id": observation_id,
    "session_id": session_id,
    "workstream_id": workstream_id,
    "event_id": event_id,
    "task_id": task_id,
    "candidate_approve_id": candidate_ids[0],
    "candidate_reject_id": candidate_ids[1],
    "candidate_edit_id": candidate_ids[2],
    "memory_id": memory_id,
}
with open(output_path, "w", encoding="utf-8") as handle:
    json.dump(fixture, handle)
PY

fixture_id() {
  json_field "$TMP_BASE/fixture.json" "$1"
}

request_json_no_auth "capabilities_unauthorized" "/api/v1/capabilities" "401" "error"
request_json_no_auth "health_unauthorized" "/api/v1/health" "401" "error"
request_json "health" "/api/v1/health" "200" "health"
request_json "status" "/api/v1/status" "200" "status"
request_json "status_cached" "/api/v1/status" "200" "status"
request_json "status_refresh" "/api/v1/status?refresh=true" "200" "status"
python3 - "$TMP_BASE/status.json" "$TMP_BASE/status_cached.json" "$TMP_BASE/status_refresh.json" <<'PY'
import json
import sys

first, cached, refresh = [json.load(open(path, "r", encoding="utf-8")) for path in sys.argv[1:]]

def require(condition, message):
    if not condition:
        raise SystemExit(message)

require(first["cache"]["hit"] is False, "first status response should not be a cache hit")
require(first["cache"]["stale"] is False, "first status response should not be stale")
require(cached["cache"]["hit"] is True, "second status response should be a cache hit")
require(cached["cache"]["stale"] is False, "cached status response should not be stale")
require(refresh["cache"]["hit"] is False, "refresh status response should bypass cache")
require(refresh["cache"]["stale"] is False, "refresh status response should not be stale")
PY
request_json "capabilities" "/api/v1/capabilities" "200" "capabilities"

for resource in observations sessions workstreams events tasks; do
  case "$resource" in
    observations) resource_id="$(fixture_id observation_id)" ;;
    sessions) resource_id="$(fixture_id session_id)" ;;
    workstreams) resource_id="$(fixture_id workstream_id)" ;;
    events) resource_id="$(fixture_id event_id)" ;;
    tasks) resource_id="$(fixture_id task_id)" ;;
  esac
  request_json "${resource}_list" "$(endpoint_path "${resource}_list")" "200" "resource_list"
  request_json "${resource}_detail" "$(endpoint_path "${resource}_detail" "$resource_id")" "200" "resource_detail"
done

CANDIDATE_APPROVE_ID="$(fixture_id candidate_approve_id)"
CANDIDATE_REJECT_ID="$(fixture_id candidate_reject_id)"
CANDIDATE_EDIT_ID="$(fixture_id candidate_edit_id)"
request_json "candidate_detail" \
  "$(endpoint_path candidate_detail "$CANDIDATE_APPROVE_ID")" "200" "candidate_detail"
request_json "candidate_evidence" \
  "$(endpoint_path candidate_evidence "$CANDIDATE_APPROVE_ID")" "200" "candidate_detail"
request_json_post "candidate_safe_approve" \
  "$(endpoint_path candidate_review_safe_approve "$CANDIDATE_APPROVE_ID")" \
  "200" "safe_mutation" \
  '{"reason":"native smoke approve","expected_version":1,"idempotency_key":"smoke-candidate-approve"}'
request_json_post "candidate_safe_reject" \
  "$(endpoint_path candidate_review_safe_reject "$CANDIDATE_REJECT_ID")" \
  "200" "safe_mutation" \
  '{"reason":"native smoke reject","expected_version":1,"idempotency_key":"smoke-candidate-reject"}'
request_json_post "candidate_safe_edit" \
  "$(endpoint_path candidate_review_safe_edit "$CANDIDATE_EDIT_ID")" \
  "200" "safe_mutation" \
  '{"reason":"native smoke edit","expected_version":1,"idempotency_key":"smoke-candidate-edit","text":"Edited native smoke candidate"}'

MEMORY_ID="$(fixture_id memory_id)"
request_json "memory_governance_detail" \
  "$(endpoint_path memory_detail "$MEMORY_ID")" "200" "memory_detail"
MEMORY_VERSION="$(json_field "$TMP_BASE/memory_governance_detail.json" version)"
request_json_post "memory_archive" \
  "$(endpoint_path memory_archive "$MEMORY_ID")" "200" "safe_mutation" \
  "{\"reason\":\"native smoke archive\",\"expected_version\":${MEMORY_VERSION},\"idempotency_key\":\"smoke-memory-archive\"}"
ARCHIVE_VERSION="$(json_field "$TMP_BASE/memory_archive.json" version)"
request_json_post "memory_restore" \
  "$(endpoint_path memory_restore "$MEMORY_ID")" "200" "safe_mutation" \
  "{\"reason\":\"native smoke restore\",\"expected_version\":${ARCHIVE_VERSION},\"idempotency_key\":\"smoke-memory-restore\"}"
request_code "memory_delete_absent" "DELETE" "$(endpoint_path memory_detail "$MEMORY_ID")" "405"

request_json "memories" "/api/v1/memories?limit=1" "200" "list"
request_json "memories_list_alias" "/api/v1/memories/list?limit=1" "200" "list"
request_json "search" "/api/v1/search?query=remem&limit=1" "200" "search"
request_json "legacy_raw_search" "/api/v1/search?query=legacy%20raw%20smoke%20sentinel&limit=1" "200" "legacy_raw_search"
request_json "candidates" "/api/v1/candidates?limit=1" "200" "list"
request_json "graph" "/api/v1/graph?limit=1" "200" "graph"
request_json "stats" "/api/v1/stats" "200" "stats"
request_json_post "user_recall" "/api/v1/user/recall" "200" "user_recall" \
  '{"query":"native API recall smoke","project":"/tmp/remem-smoke","limit":3,"budget_chars":1000}'
request_json "legacy_memory_not_found" "/api/v1/memory?id=999999999" "404" "error"
request_json "memory_detail_not_found" "/api/v1/memories/999999999" "404" "error"
assert_no_token_leak
assert_no_idempotency_key_leak

echo "native web API smoke passed on ${BASE_URL}"
