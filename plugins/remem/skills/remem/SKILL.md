---
name: remem
description: Use when the user asks Codex to recall prior project context, save durable decisions or bug fixes, inspect remem memory health, or activate remem automatic memory hooks from the Codex plugin.
---

# Remem

Remem gives Codex durable project memory through the `remem` MCP server. Use it when the answer depends on prior sessions, project preferences, saved decisions, workstreams, or memory health.

## Retrieval

- Search remem before answering questions about prior project context, remembered preferences, older bug fixes, workstreams, or "what did we decide before".
- Use compact `search` results first, then call `get_observations` for selected IDs when exact content is needed.
- Use `search_raw` only when curated search misses an exact phrase or chat transcript detail.
- Treat memory as evidence, not current truth. Verify live repo, GitHub, filesystem, and command output when the fact can drift.

## Saving Memory

Save a durable memory after:

- architecture decisions, including what was chosen and what was rejected
- bug fixes with a verified root cause, fix, and prevention
- important project discoveries with future implications
- explicit user preferences that should affect future sessions

Use the active repo/workspace `project` plus a stable `topic_key` so repeated saves update the same memory instead of creating duplicates.
For project memories, pass `project` as the active repo/workspace path and pass
`branch` when it is known. Use `scope: "global"` only for explicit cross-project
user preferences.

## Activation

The plugin exposes MCP tools as soon as Codex loads `.mcp.json` and the runtime
manager can resolve a matching `remem` binary. Automatic context injection and
Stop summarization require explicit activation. The plugin does not ship
auto-loaded hook definitions because Codex hook runners do not always provide a
plugin root environment. Activation writes verified host-level hooks through
`remem install --target codex --hooks-only`.

For a local repo checkout, activate with:

```bash
cargo build --release
node plugins/remem/scripts/remem-runtime.js install
node plugins/remem/scripts/activate-codex.js --dry-run
node plugins/remem/scripts/activate-codex.js
remem doctor
remem status
```

If the plugin cannot find the binary, build or explicitly point at remem first:

```bash
cargo build --release
REMEM_BINARY="$PWD/target/release/remem" node plugins/remem/scripts/activate-codex.js --dry-run
```

The wrapper rejects stale remem binaries by default. Use `REMEM_ALLOW_VERSION_MISMATCH=1` only for explicit local debugging.

## Local App

For local GUI testing from this repo, run:

```bash
cargo build --release
node plugins/remem/scripts/remem-runtime.js install
node plugins/remem/apps/remem/server.js --port 5577
```

The app exposes runtime status, memory counts, search/detail, explicit save, and
activation dry-run views. Treat this as a local Apps SDK surface until a real
`.app.json` app id is available.

After installing or updating the plugin, start a new Codex thread so skills and MCP tools are reloaded.
