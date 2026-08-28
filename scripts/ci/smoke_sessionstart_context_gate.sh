#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd -P)"
scratch_dir="$(mktemp -d "${TMPDIR:-/tmp}/remem-sessionstart-smoke.XXXXXX")"

cleanup() {
  if [[ -n "${scratch_dir:-}" && -d "${scratch_dir}" ]]; then
    rm -rf -- "${scratch_dir}"
  fi
}
trap cleanup EXIT

umask 077
smoke_home="${scratch_dir}/home"
data_dir="${scratch_dir}/data"
config_path="${scratch_dir}/config.toml"
mkdir -p -- "${smoke_home}" "${data_dir}"

cd -- "${repo_root}"
binary="$(
  cargo build --locked --bin remem --message-format=json-render-diagnostics \
    | python3 -c '
import json
import sys

executables = []
for line in sys.stdin:
    artifact = json.loads(line)
    if (
        artifact.get("reason") == "compiler-artifact"
        and artifact.get("target", {}).get("name") == "remem"
        and artifact.get("executable")
    ):
        executables.append(artifact["executable"])
if len(executables) != 1:
    raise SystemExit(
        f"expected one current remem executable from cargo, found {len(executables)}"
    )
print(executables[0])
'
)"
if [[ ! -x "${binary}" ]]; then
  echo "SessionStart smoke failed: built remem binary is not executable at ${binary}" >&2
  exit 1
fi

common_env=(
  env
  -i
  "PATH=/usr/bin:/bin"
  "LANG=C"
  "LC_ALL=C"
  "HOME=${smoke_home}"
  "REMEM_DATA_DIR=${data_dir}"
  "REMEM_CONFIG=${config_path}"
  "REMEM_CONTEXT_HOST=codex-cli"
  "REMEM_CONTEXT_GATE=strict"
  "REMEM_CONTEXT_GATE_HOSTS=codex-cli"
  "REMEM_CONTEXT_BUNDLE_RENDER_MODE=bundle"
  "REMEM_CONTEXT_DEBUG=0"
  "REMEM_CONTEXT_GATE_RETENTION_DAYS=30"
)

if ! "${common_env[@]}" "${binary}" encrypt >"${scratch_dir}/encrypt.stdout"; then
  echo "SessionStart smoke failed: could not initialize the encrypted store" >&2
  exit 1
fi

transcript_path="${scratch_dir}/gate-smoke.jsonl"
payload="$(python3 -c \
  'import json, sys; print(json.dumps({"session_id": "gate-smoke", "cwd": sys.argv[1], "transcript_path": sys.argv[2]}, separators=(",", ":")))' \
  "${repo_root}" "${transcript_path}")"
context_argv=(context)

if ! printf '%s' "${payload}" \
  | "${common_env[@]}" "${binary}" "${context_argv[@]}" >"${scratch_dir}/first.out"; then
  echo "SessionStart smoke failed: first context invocation returned an error" >&2
  exit 1
fi
if [[ ! -s "${scratch_dir}/first.out" ]]; then
  echo "SessionStart smoke failed: first context invocation emitted no bytes" >&2
  exit 1
fi

if ! printf '%s' "${payload}" \
  | "${common_env[@]}" "${binary}" "${context_argv[@]}" >"${scratch_dir}/second.out"; then
  echo "SessionStart smoke failed: second context invocation returned an error" >&2
  exit 1
fi
if [[ -s "${scratch_dir}/second.out" ]]; then
  second_bytes="$(wc -c <"${scratch_dir}/second.out" | tr -d '[:space:]')"
  echo "SessionStart smoke failed: unchanged second invocation emitted ${second_bytes} bytes" >&2
  exit 1
fi

first_bytes="$(wc -c <"${scratch_dir}/first.out" | tr -d '[:space:]')"
echo "SessionStart context gate smoke passed: first=${first_bytes} bytes, second=0 bytes"
