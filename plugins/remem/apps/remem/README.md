# Remem App Surface

Local Remem GUI prototype for the Codex plugin.

This app surface provides:

- runtime status: expected version, schema version, selected binary, plugin data path
- project memory dashboard: memory, observation, raw archive, and queue counts
- search: compact results with detail drill-down
- save memory: explicit decision, bugfix, architecture, discovery, or preference save
- governance: stale, reject, or delete dry-run preview with affected IDs and status transitions
- activation: hooks-only dry-run plan without writing Codex config

Run locally:

```bash
cargo build --release
node plugins/remem/scripts/remem-runtime.js install
node plugins/remem/apps/remem/server.js --port 5577
```

Open `http://127.0.0.1:5577/`.

The server also exposes a JSON-RPC MCP-style endpoint at `/mcp` with tool
descriptors and a `ui://remem/dashboard.html` resource. The plugin manifest
does not point at `.app.json` yet because Codex plugin validation only accepts
real app IDs there. Add `.app.json` after a real Apps SDK app or connector id
exists.
