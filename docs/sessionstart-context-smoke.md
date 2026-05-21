# SessionStart Context Smoke Matrix

Use these read-only commands after context compiler changes:

```bash
REMEM_CONTEXT_HOST=codex-cli cargo run --quiet -- context --cwd /Users/lifcc/Desktop/code/AI/tools/remem
REMEM_CONTEXT_HOST=claude-code cargo run --quiet -- context --cwd /Users/lifcc/Desktop/code/AI/tools/remem
REMEM_CONTEXT_DEBUG=1 REMEM_CONTEXT_HOST=codex-cli cargo run --quiet -- context --cwd /Users/lifcc/Desktop/code/AI/tools/remem
```

Expected checks:

- Normal output has no `## Debug Trace` section.
- Footer includes host, branch, per-section chars, approximate tokens, and truncation status.
- Project preferences render by default; global preferences require `REMEM_CONTEXT_PREFERENCE_GLOBAL_LIMIT`.
- Long session requests are truncated inside the Sessions section.
- Total truncation keeps the truncation marker and stats footer when both fit.
