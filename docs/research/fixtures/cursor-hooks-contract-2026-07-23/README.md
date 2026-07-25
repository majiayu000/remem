# Cursor hook probe v2 evidence bundle

This directory contains replacement-value evidence from the GH-822 Cursor IDE
3.12.17 real-host probe. It includes no real account, path, UUID, model,
transcript text, token count, or stable payload/config hash.

## Files

- `payload-shapes.synthetic.json`: observed event keys and JSON types plus safe
  event/tool/status enum values.
- `transcript.synthetic.jsonl`: representative observed JSONL record grammar,
  including a tool call and cancelled-turn boundary.
- `transcript-metrics.synthetic.tsv`: real append-boundary sizes and row counts
  from the synthetic-prompt session; no path or content.
- `probe.sh`: bounded stdin capture and context marker generator.
- `hooks.template.json`: placeholder-only probe entries used to observe the
  covered events.

## Reproduction and restore

1. Record the Cursor version and start from a clean, read-only workspace.
2. Create private run and backup directories with mode `0700`.
3. Back up `~/.cursor/hooks.json`, store its SHA-256 privately, and validate it
   with `jq`.
4. Validate absolute probe paths against `/[A-Za-z0-9._/-]+/` before replacing
   placeholders in `hooks.template.json`.
5. Append probe entries to existing event arrays. Preserve every foreign entry
   and order. Validate the staged JSON, then atomically install it.
6. Through Cursor UI, run only bounded benign prompts: no-tool response,
   README read plus nonexistent synthetic path, read-only shell status,
   list-only MCP, one read-only multitask subagent, cancelled text generation,
   and `/summarize`.
7. Inspect raw payloads only through key/type/ordering/cardinality summaries.
8. Restore the backup immediately after the final event. Verify byte equality
   with the private pre-probe SHA-256 and re-run `jq`.
9. Delete private payloads and backup after sanitization and restore
   verification.

The template intentionally does not include the temporary non-zero/timeout
commands. Those exact commands and their observed behavior are documented in
the report.
