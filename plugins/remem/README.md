# Remem Codex Plugin

This directory contains a local Codex plugin wrapper for remem.

The plugin exposes `remem mcp` to Codex and provides a Remem skill for retrieval, saving, governance, and activation workflows. It does not silently install hooks. Automatic SessionStart context injection and Stop summarization require an explicit activation step.

This is a development foundation, not the final plugin experience. A complete
plugin should manage its own version-matched remem runtime under plugin storage
and should work even when the user has not installed remem separately. See
`../../docs/spec-codex-plugin-complete-design.md` for the target design.

## Local Install

From the repository root:

```bash
codex plugin marketplace add .
codex plugin add remem@remem-local
```

Start a new Codex thread after installing the plugin.

## Binary Resolution

The MCP wrapper looks for the remem binary in this order:

1. `REMEM_BINARY`
2. `target/release/remem`
3. `target/debug/remem`
4. `remem` on `PATH`

By default, the selected binary must report the same version as the plugin
manifest. Set `REMEM_ALLOW_VERSION_MISMATCH=1` only for explicit local
debugging.

Build from source when testing directly from this repository:

```bash
cargo build --release
```

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
remem install --target codex
```

That command updates `~/.codex/config.toml`, `~/.codex/hooks.json`, and `~/.remem/config.toml`.
