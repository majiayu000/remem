# Remem Codex Plugin

This directory contains a local Codex plugin wrapper for remem.

The plugin exposes `remem mcp` to Codex and provides a Remem skill for retrieval, saving, governance, and activation workflows. It does not silently install hooks. Automatic SessionStart context injection and Stop summarization require an explicit activation step.

This is a development foundation, not the final Apps SDK GUI experience. The
plugin now manages a version-matched local runtime under plugin storage for
local checkout testing, so it no longer needs to select a stale `remem` from
`PATH`. See `../../docs/spec-codex-plugin-complete-design.md` for the target
design.

## Local Install

From the repository root:

```bash
codex plugin marketplace add .
codex plugin add remem@remem-local
```

Start a new Codex thread after installing the plugin.

## Runtime Resolution

The MCP wrapper resolves the remem runtime in this order:

1. `REMEM_BINARY`
2. `PLUGIN_DATA/bin/remem` or `REMEM_PLUGIN_DATA/bin/remem`
3. `target/release/remem`
4. `target/debug/remem`
5. `remem` on `PATH`, reported but not silently adopted

When a matching repo binary exists, the runtime manager copies it into plugin
storage. PATH binaries are never adopted silently; use `REMEM_BINARY` for
explicit development overrides or `--adopt-path` for a deliberate local copy.
By default, the selected binary must report the same version as the plugin
manifest. Set `REMEM_ALLOW_VERSION_MISMATCH=1` only for explicit local debugging.

Build from source when testing directly from this repository:

```bash
cargo build --release
node plugins/remem/scripts/remem-runtime.js install
node plugins/remem/scripts/remem-runtime.js status
```

Release download support is intentionally checksum-gated. Until a matching
release manifest exists for the plugin version, fresh installs outside a local
checkout must provide `REMEM_BINARY` or use a published plugin build that
includes checked runtime assets.

Plugin MCP startup leaves the server cwd as the active Codex workspace. The
wrapper is invoked by plugin-root path so repo-scoped memory operations can use
the caller workspace instead of the plugin checkout.

## Hook Activation

MCP tools work without hook activation. To enable automatic memory injection and Stop summarization for Codex:

```bash
node plugins/remem/scripts/activate-codex.js --dry-run
node plugins/remem/scripts/activate-codex.js
remem doctor
remem status
```

The activation script delegates to:

```bash
remem install --target codex --hooks-only
```

That command enables Codex hooks without adding another global `remem` MCP
server entry, because the plugin already provides MCP through `.mcp.json`.
