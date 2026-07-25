# SessionStart Context Smoke Matrix

Use these read-only commands after context compiler changes:

```bash
REMEM_CONTEXT_HOST=codex-cli cargo run --quiet -- context --cwd /Users/lifcc/Desktop/code/AI/tools/remem
REMEM_CONTEXT_HOST=claude-code cargo run --quiet -- context --cwd /Users/lifcc/Desktop/code/AI/tools/remem
REMEM_CONTEXT_DEBUG=1 REMEM_CONTEXT_HOST=codex-cli cargo run --quiet -- context --cwd /Users/lifcc/Desktop/code/AI/tools/remem
```

Codex duplicate-injection gate:

```bash
tmpdir="$(mktemp -d)"
printf '{"session_id":"gate-smoke","cwd":"%s","transcript_path":"/tmp/remem-gate-smoke.jsonl"}' "$PWD" \
  | REMEM_DATA_DIR="$tmpdir" REMEM_CONTEXT_HOST=codex-cli cargo run --quiet -- context | wc -c
printf '{"session_id":"gate-smoke","cwd":"%s","transcript_path":"/tmp/remem-gate-smoke.jsonl"}' "$PWD" \
  | REMEM_DATA_DIR="$tmpdir" REMEM_CONTEXT_HOST=codex-cli cargo run --quiet -- context | wc -c
rm -rf "$tmpdir"
```

Expected checks:

- Normal output has no `## Debug Trace` section.
- Footer includes host, branch, per-section chars, relevance state/k/threshold/drop counts, approximate tokens, and truncation status.
- `REMEM_CONTEXT_RELEVANCE_K=0` restores legacy governed-section selection.
- Project preferences render by default; global preferences require `REMEM_CONTEXT_PREFERENCE_GLOBAL_LIMIT`.
- Long session requests are truncated inside the Sessions section.
- Total truncation keeps the truncation marker and stats footer when both fit.
- Codex gate smoke emits bytes on the first command and `0` bytes on the second unchanged same-session command.
