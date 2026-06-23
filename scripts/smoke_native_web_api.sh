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
elif shape == "list":
    require_keys(payload, ["data", "meta"])
    require(isinstance(payload["data"], list), "data should be an array")
    require_keys(payload["meta"], ["count", "total", "limit", "offset", "has_more", "next_offset"])
elif shape == "search":
    require_keys(payload, ["data", "meta"])
    require(isinstance(payload["data"], list), "data should be an array")
    require_keys(payload["meta"], ["count", "has_more", "limit", "offset"])
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

assert_no_token_leak() {
  [[ -n "$TOKEN" ]] || return 0
  if grep -Fq -- "$TOKEN" "$TMP_BASE/remem-api.log" "$TMP_BASE"/*.json 2>/dev/null; then
    fail "API token appeared in smoke output"
  fi
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
request_json "memories" "/api/v1/memories?limit=1" "200" "list"
request_json "memories_list_alias" "/api/v1/memories/list?limit=1" "200" "list"
request_json "search" "/api/v1/search?query=remem&limit=1" "200" "search"
request_json "candidates" "/api/v1/candidates?limit=1" "200" "list"
request_json "graph" "/api/v1/graph?limit=1" "200" "graph"
request_json "stats" "/api/v1/stats" "200" "stats"
request_json_post "user_recall" "/api/v1/user/recall" "200" "user_recall" \
  '{"query":"native API recall smoke","project":"/tmp/remem-smoke","limit":3,"budget_chars":1000}'
request_json "legacy_memory_not_found" "/api/v1/memory?id=1" "404" "error"
request_json "memory_detail_not_found" "/api/v1/memories/1" "404" "error"
assert_no_token_leak

echo "native web API smoke passed on ${BASE_URL}"
