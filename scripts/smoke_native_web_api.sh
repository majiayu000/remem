#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PORT="${REMEM_API_SMOKE_PORT:-5567}"
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
  TARGET_DIR="$(cargo metadata --no-deps --format-version 1 | python3 -c 'import json, sys; print(json.load(sys.stdin)["target_directory"])')"
  REMEM_BIN="$TARGET_DIR/debug/remem"
  cargo build --bin remem
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

if shape == "status":
    require_keys(payload, ["version"])
elif shape == "capabilities":
    require_keys(payload, ["version", "schema_version", "api_version", "features", "endpoints"])
    require(payload["features"].get("graph") is True, "graph feature should be true")
    require(payload["features"].get("candidate_review") is True, "candidate_review feature should be true")
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
      body="$TMP_BASE/ready-status.json"
      code="$(curl -sS -o "$body" -w '%{http_code}' \
        -H "Authorization: Bearer ${TOKEN}" \
        "${BASE_URL}/api/v1/status" || true)"
      if [[ "$code" == "200" ]]; then
        validate_json "status" "status" "$body"
        break
      fi
    fi
  fi
  sleep 0.25
done

[[ -n "$TOKEN" ]] || fail "API token was not created"

request_json_no_auth "capabilities_unauthorized" "/api/v1/capabilities" "401" "error"
request_json "status" "/api/v1/status" "200" "status"
request_json "capabilities" "/api/v1/capabilities" "200" "capabilities"
request_json "memories" "/api/v1/memories?limit=1" "200" "list"
request_json "memories_list_alias" "/api/v1/memories/list?limit=1" "200" "list"
request_json "search" "/api/v1/search?query=remem&limit=1" "200" "search"
request_json "candidates" "/api/v1/candidates?limit=1" "200" "list"
request_json "graph" "/api/v1/graph?limit=1" "200" "graph"
request_json "stats" "/api/v1/stats" "200" "stats"
request_json "legacy_memory_not_found" "/api/v1/memory?id=1" "404" "error"
request_json "memory_detail_not_found" "/api/v1/memories/1" "404" "error"
assert_no_token_leak

echo "native web API smoke passed on ${BASE_URL}"
