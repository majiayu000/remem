# Remem App Surface

Local Remem GUI prototype for the Codex plugin.

This app surface provides:

- runtime status: expected version, schema version, selected binary, plugin data path
- project memory dashboard: memory, observation, raw archive, and queue counts
- search: compact results with detail drill-down
- save memory: explicit decision, bugfix, architecture, discovery, or preference save
- governance: stale, reject, or delete dry-run preview with affected IDs and status transitions
- current-state resolution: stable state keys with current/conflict/history metadata
- traceability: commit lookup and session-to-commit links
- activation: hooks-only dry-run plan without writing Codex config

## Surface map

| Surface | App tool | HTTP route | Remem backend |
| --- | --- | --- | --- |
| Dashboard | `remem_dashboard` | `GET /api/status` | `remem status --json`, `remem doctor --json`, runtime inspection |
| Search | `remem_search` | `GET /api/search` | local Remem API `GET /api/v1/search` |
| Detail | `remem_get_memory` | `GET /api/memory` | local Remem API `GET /api/v1/memory` |
| Save | `remem_save_memory` | `POST /api/save` | local Remem API `POST /api/v1/memories` |
| Governance preview | `remem_governance_preview` | `POST /api/governance-preview` | `remem govern --dry-run --json` |
| Current state | `remem_current_state` | `GET /api/current-state` | `remem current --json` |
| Commit lookup | `remem_commit_lookup` | `GET /api/commit` | `remem commit show --json` |
| Session commits | `remem_session_commits` | `GET /api/session-commits` | `remem commit session --json` |
| Activation plan | `remem_activation_plan` | `GET /api/activation-plan` | `remem install --target codex --hooks-only --dry-run` |

Workstream mutation and chronological timeline browsing now have machine-readable
Rust CLI bridges (`remem workstreams ... --json`, `remem timeline ... --json`)
plus MCP-native tools. Add them to this app through those JSON bridges; do not
parse human CLI output for those surfaces. Workstream updates must pass an
explicit project and `--confirm`.

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
