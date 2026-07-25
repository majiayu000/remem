#!/bin/sh
set -eu

event=${1:?event name required}
run_dir=${2:?private run directory required}
mode_file=${3:?context mode file required}
umask 077
mkdir -p "$run_dir"

stamp=$(date -u '+%Y%m%dT%H%M%S')
payload="$run_dir/${stamp}-$$-${event}.json"
cat >"$payload"
printf '%s\t%s\n' "$event" "$(basename "$payload")" >>"$run_dir/events.tsv"

transcript_path=$(jq -r 'select(.transcript_path | type == "string") | .transcript_path' "$payload")
if [ -n "$transcript_path" ] && [ -f "$transcript_path" ]; then
  size=$(stat -f '%z' "$transcript_path")
  lines=$(wc -l <"$transcript_path")
  lines=$(printf '%s' "$lines" | tr -d ' ')
  printf '%s\t%s\t%s\n' "$event" "$size" "$lines" >>"$run_dir/transcript-metrics.tsv"
fi

case "$event" in
  sessionStart)
    mode=$(cat "$mode_file")
    python3 - "$mode" <<'PY'
import json
import sys

mode = sys.argv[1]
counts = {"small": 0, "medium": 16384, "large": 65536}
if mode not in counts:
    raise SystemExit(f"unsupported context mode: {mode}")
marker = f"GH822_{mode.upper()}_MULTIBYTE_SUFFIX_20260716"
body = ("界" * counts[mode]) + marker
print(json.dumps({"additional_context": body}, ensure_ascii=False, separators=(",", ":")))
PY
    ;;
  postToolUse)
    printf '%s\n' '{"additional_context":"GH822_POSTTOOLUSE_CONTEXT_20260716"}'
    ;;
esac
