# Cursor hook probe evidence bundle

This directory publishes replacement-value fixtures and the bounded probe
harness used for GH-822 follow-up work. It intentionally contains no real
account, path, UUID, model name, token count, transcript text, or stable hash.

## Files

- `session-start.synthetic.json`: observed Cursor 3.6.31 `sessionStart` keys and
  JSON types, with every identifying value replaced.
- `stop.synthetic.json`: observed successful `stop` keys and JSON types, with
  every identifying or usage value replaced.
- `transcript.synthetic.jsonl`: observed two-row JSONL grammar with synthetic
  text.
- `probe.sh`: stdin capture, transcript-size sampling, and bounded
  short/medium/long multibyte `additional_context` generator.
- `hooks.template.json`: all probe entries. Placeholder paths must be validated
  and replaced before use; existing hook entries must be preserved when
  merging.

No `preToolUse`, `postToolUse`, `postToolUseFailure`, MCP, subagent, cancelled
Stop, or `preCompact` fixture is published because the real host did not emit
those events during the completed run. Inventing such fixtures would turn an
open contract question into false evidence.

## Reproduction

1. Confirm Cursor is running and record its version.
2. Create a private directory outside the repository with mode `0700`.
3. Copy `~/.cursor/hooks.json` into a separate `0700` backup directory and
   record its SHA-256 locally. Do not publish the fingerprint or config body.
4. Select absolute local paths for `<probe-script>`, `<private-run-dir>`, and
   `<context-mode-file>`. Before replacing the placeholders, validate every
   path with the following command. This template deliberately rejects spaces
   and shell metacharacters rather than trying to quote arbitrary replacement
   text:

   ```sh
   python3 - "$probe_script" "$private_run_dir" "$context_mode_file" <<'PY'
   import re
   import sys

   pattern = re.compile(r"/[A-Za-z0-9._/-]+")
   for path in sys.argv[1:]:
       if pattern.fullmatch(path) is None:
           raise SystemExit(f"unsafe probe path: {path!r}")
   PY
   ```

   Only after that validation succeeds, copy `hooks.template.json` to a
   temporary file and replace the three placeholders. The mode file contains
   exactly `small`, `medium`, or `large`.
5. Merge each entry by appending to the matching event array in the existing
   user config. Do not replace existing entries. Validate the result with
   `jq -e .` before atomically replacing the config.
6. Through normal Cursor UI only, create a new foreground Agent session. Ask it
   to return the synthetic marker already present in initial context without
   using tools or reading files.
7. For the remaining GH-822 gates, use only benign UI requests:
   - read/search a public README;
   - read one explicitly nonexistent synthetic relative path;
   - call a status/list-only MCP tool if one is already available;
   - create and end one read-only background agent;
   - exchange several short synthetic messages;
   - cancel one ordinary generation;
   - use a normal compaction UI command only if the product exposes one.
8. Change the mode file only between new sessions. The generated context-body
   sizes are deterministic:
   - `small`: marker only, 37 UTF-8 bytes;
   - `medium`: 16,384 copies of `界` plus the marker, 49,190 UTF-8 bytes;
   - `large`: 65,536 copies of `界` plus the marker, 196,645 UTF-8 bytes.
   These are bounded acceptance probes, not a resource-exhaustion search and
   not evidence of an exact product limit.
9. Inspect private payloads locally by outputting keys, JSON types, ordering,
   lengths, and synthetic replacements only. Never print raw account, path,
   identifier, model, tool content, transcript text, or usage values into a
   repository artifact.
10. Restore the backup immediately after the final event or at the first UI or
    probe failure. Verify the restored SHA-256 equals the private pre-probe
    value and `jq -e .` succeeds.
11. Delete captured payloads, transcript snapshots, mode file, and backup after
    sanitization and restore verification.

## Fixture semantics

Placeholder strings such as `<session-redacted>` preserve only the fact that
the real value was a string. Synthetic numeric values are zero and do not
preserve private usage fingerprints. `workspace_roots` preserves observed
array cardinality but not a filesystem shape. The transcript preserves key
nesting, row ordering, and roles only.

The completed v1 run proves only the three synthetic fixtures in this bundle.
The expanded harness was registered for a second run, but Cursor exposed no
controllable window to the Computer Use channel and emitted zero events. The
original config was restored before the attempt was stopped.
