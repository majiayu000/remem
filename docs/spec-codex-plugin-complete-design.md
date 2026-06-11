# Remem Complete Codex Plugin Design

## Summary

The current Remem Codex plugin is a wrapper. It exposes a skill and a bundled
MCP server config, but it still depends on a separately installed `remem`
binary. That makes the plugin useful for local development, but weak as a
product: installing the plugin alone does not create a working memory system.

The complete plugin should be a self-contained local runtime first, and an
Apps SDK GUI app second.

The target user experience is:

1. Install Remem from the Codex plugin directory.
2. Start a new thread.
3. Ask Remem to check memory status, search memory, save a memory, or open the
   memory dashboard.
4. If the runtime is missing, the plugin installs or downloads a matching
   Remem runtime into plugin-managed storage.
5. Automatic context injection and session summarization remain explicit,
   reviewable, and trusted by the user before hooks run.

## Problem

The current plugin does not satisfy the expected plugin contract. Official
plugins observed locally follow one of these useful shapes:

- Connector plugins, such as GitHub and Gmail, provide an app connector via
  `.app.json` and use skills as workflow guidance.
- Runtime plugins, such as Documents and Spreadsheets, bundle their own
  scripts and resources so installation immediately enables useful work.
- Local control plugins, such as Browser and Chrome, bundle runtime scripts,
  docs, assets, and host integration code.

The current Remem plugin is different:

```text
Codex plugin -> skill + .mcp.json -> node wrapper -> system remem binary
```

If the machine has no compatible `remem` binary, the plugin cannot do the work
it advertises. This creates a bad first-run experience and makes the plugin
feel like an empty shell.

## Goals

- Installing the plugin alone should be enough to use manual memory features.
- The plugin should not rely on `remem` being installed on `PATH`.
- The plugin should manage a version-matched runtime under plugin-controlled
  storage.
- Existing global Remem installs should be detected and adopted only with an
  explicit user choice.
- Hook-based automatic capture should be opt-in and reviewable.
- The design should keep local memory data local by default.
- The design should support a future GUI dashboard through Apps SDK without
  blocking the local runtime MVP.

## Non-goals

- Do not silently modify `~/.codex/config.toml` or `~/.codex/hooks.json` during
  plugin installation.
- Do not depend on a user-managed Homebrew, cargo, npm, or manual binary
  install for the baseline plugin experience.
- Do not make the Apps SDK app id a prerequisite for the local plugin runtime.
- Do not ship destructive memory governance without dry-run and confirmation.

## Architecture

### Layer 1: Codex plugin package

The package remains under `plugins/remem`:

```text
plugins/remem/
  .codex-plugin/plugin.json
  .mcp.json
  README.md
  assets/
  hooks/hooks.json
  scripts/
    remem-mcp.js
    remem-hook.js
    remem-runtime.js
    remem-download.js
    remem-status.js
  skills/remem/SKILL.md
  runtimes/remem-releases.json
```

Responsibilities:

- present Remem in the Codex plugin directory
- expose Remem skill workflows
- expose the Remem MCP server
- manage runtime discovery, download, checksum validation, and version checks
- provide optional plugin-bundled hooks for automatic memory capture

### Layer 2: plugin-managed runtime

The plugin should prefer a plugin-managed binary:

```text
$PLUGIN_DATA/bin/remem
```

Resolution order:

1. `REMEM_BINARY`, for explicit development override only.
2. `$PLUGIN_DATA/bin/remem`, installed by the plugin runtime manager.
3. repository `target/release/remem`, for local checkout development.
4. system `remem` on `PATH`, only as an adoptable external install.

The default production path should be `PLUGIN_DATA`, not `PATH`. A stale
system binary should be reported as an external install, not selected silently.

Runtime metadata lives in:

```text
$PLUGIN_DATA/runtime.json
```

Example:

```json
{
  "version": "0.5.17",
  "schema": 34,
  "source": "github-release",
  "binary": "/Users/example/.codex/plugin-data/remem/bin/remem",
  "installed_at": "2026-06-11T00:00:00Z",
  "sha256": "..."
}
```

### Layer 3: MCP server

The plugin MCP wrapper should guarantee that `remem mcp` starts with a
version-matched runtime.

Startup behavior:

1. Read plugin version from `.codex-plugin/plugin.json`.
2. Resolve or install a matching runtime.
3. Run `remem doctor --json` and `remem status --json` when requested by tools,
   not on every MCP startup.
4. Start `remem mcp` over stdio.
5. Fail closed with a structured message if runtime installation or checksum
   validation fails.

The existing Rust MCP tools remain the core model interface:

- `search`
- `get_observations`
- `search_raw`
- `current_state`
- `timeline`
- `lookup_commit`
- `commits_for_session`
- `save_memory`
- `govern_memory`
- `timeline_report`
- `workstreams`
- `update_workstream`

Add plugin-facing utility tools or commands:

- `plugin_status`: show runtime, data, hook, and schema state
- `ensure_runtime`: install or repair the plugin-managed runtime
- `activation_plan`: show hook changes before activation
- `activate_hooks`: enable automatic capture after explicit approval

### Layer 4: hooks

Automatic memory capture should use plugin-bundled hooks where possible:

```text
plugins/remem/hooks/hooks.json
```

The hook commands should call a plugin script, not a global binary:

```text
node ${PLUGIN_ROOT}/scripts/remem-hook.js session-start
node ${PLUGIN_ROOT}/scripts/remem-hook.js stop
```

`remem-hook.js` resolves the plugin-managed runtime and then delegates to:

```text
remem session-init
remem summarize
```

Important behavior:

- Plugin hooks are not trusted automatically. The user must review and trust
  the current hook definition before they run.
- The first-run UI and skill should explain this explicitly.
- If hooks are not trusted, manual MCP tools still work.
- Hook failures must be visible in `plugin_status` and `doctor --json`.

This removes the need for the plugin to edit global Codex hook config for the
common path. The existing `remem install --target codex` path can remain as a
legacy or advanced installation mode.

### Layer 5: Apps SDK GUI

The GUI is a separate Apps SDK app surface. It should not block the local
runtime MVP.

Proposed layout:

```text
apps/remem/
  package.json
  src/server.ts
  src/tools/
  src/ui/
  src/remem-client.ts
```

The Apps SDK server provides:

- HTTP MCP endpoint at `/mcp`
- tool descriptors for Remem operations
- iframe UI resources
- structured tool results for the model
- `_meta` data for the widget

Codex plugin integration adds `.app.json` only after an app or connector id
exists:

```json
{
  "apps": {
    "remem": {
      "id": "asdk_app_..."
    }
  }
}
```

The local plugin runtime and the Apps SDK app can share the same Rust backend.
The GUI should call structured APIs, not parse human CLI output.

## First-run experience

### Fresh machine, no Remem installed

1. User installs Remem plugin from Codex.
2. User starts a new Codex thread.
3. User asks `@remem status` or invokes Remem.
4. Plugin reports `runtime_missing`.
5. User approves runtime installation.
6. Plugin downloads a platform-specific Remem binary, verifies checksum, and
   stores it in `$PLUGIN_DATA/bin/remem`.
7. Plugin initializes local data directory.
8. Manual MCP tools work immediately.
9. Plugin offers optional hook trust for automatic capture.

### Existing Remem install

1. Plugin detects system `remem`.
2. If the version matches, it offers to adopt the existing install.
3. If the version is stale, it defaults to plugin-managed runtime and reports
   the stale system path.
4. It never silently chooses a mismatched binary.

### Hook activation

1. User asks to enable automatic memory.
2. Plugin shows an activation plan.
3. User reviews hook definitions in Codex.
4. After trust, SessionStart and Stop hooks run through `remem-hook.js`.
5. `plugin_status` reports hooks as active/trusted.

## GUI product design

The Apps SDK GUI should expose the parts of Remem that users need to inspect
and control, not just a marketing panel.

### Dashboard

- runtime version and schema version
- database path and active project
- memory count, observation count, raw archive count
- pending queue health
- worker state
- hook trust and activation state
- stale binary or duplicate install warnings

### Search

- query box
- project filter
- branch filter
- memory type filter
- include stale toggle
- multi-hop toggle
- compact result list
- detail panel using `get_observations`
- raw archive fallback section clearly marked as recall evidence

### Save

- title
- memory text
- type
- project
- topic key
- scope
- branch
- related files
- local-copy status
- claim status
- result verification link

### Governance

- selected memory IDs
- action: stale, reject, delete
- dry-run preview
- explicit reason
- destructive confirmation
- audit result

### Workstreams

- active, paused, completed, abandoned filters
- next action and blockers
- status update action

### Timeline

- project timeline report
- anchor/query timeline
- commit lookup
- session-to-commit links

## Security and trust

- Runtime downloads require pinned version metadata and SHA-256 verification.
- Checksum mismatch is fatal.
- No hardcoded credentials.
- Local database paths must stay local unless the user explicitly exports or
  shares data.
- Governance mutations require dry-run first and explicit confirmation for the
  actual mutation.
- Hook activation requires explicit user trust.
- Plugin-managed runtime should not shadow `PATH` globally.
- App UI must not expose raw archive content by default without user action,
  because raw chat history can contain sensitive information.

## Implementation plan

### Phase 0: current wrapper

Already implemented in the draft plugin:

- plugin manifest
- repo marketplace entry
- Remem skill
- `.mcp.json`
- node wrapper for `remem mcp`
- explicit activation script
- stale binary guard

This phase is useful for local development but not sufficient as the final
plugin product.

### Phase 1: self-contained local runtime

Deliverables:

- `scripts/remem-runtime.js` for runtime status, install, and path resolution
- `$PLUGIN_DATA/bin/remem` or `REMEM_PLUGIN_DATA/bin/remem` install path
- local checkout adoption from `target/release/remem` or `target/debug/remem`
- `runtimes/remem-releases.json` for future platform artifacts and checksums
- tests using temp plugin data and isolated `PATH`

Acceptance:

- With no `remem` on `PATH`, installing the plugin and invoking `@remem status`
  can install or repair a runtime.
- The plugin does not select stale system binaries silently.
- MCP starts from the plugin-managed runtime.
- Release download remains checksum-gated; if no manifest entry exists for the
  plugin version, the installer fails closed with an actionable message.

### Phase 2: plugin-bundled hooks

Deliverables:

- `hooks/hooks.json`
- `scripts/remem-hook.js`
- hook trust/status reporting
- migration guidance from global `remem install --target codex`

Acceptance:

- Manual MCP works without hook trust.
- Automatic SessionStart/Stop works after hook trust.
- Hook commands resolve the plugin-managed runtime.
- No silent global config edits are required for the normal path.

### Phase 3: Apps SDK GUI

Deliverables:

- `plugins/remem/apps/remem/server.js` local app server
- `/mcp` JSON-RPC endpoint with tool descriptors
- `ui://remem/dashboard.html` resource
- runtime dashboard, search/detail, explicit save, and activation dry-run UI
- structured tool results and widget `_meta`
- Node contract tests for REST and MCP-style calls

Acceptance:

- Local app tools list through `/mcp`.
- Dashboard renders with local Remem status.
- Search, detail, save, and activation dry-run work end to end.
- `.app.json` is added only after the app id exists.

### Phase 4: distribution

Deliverables:

- signed or checksummed release artifacts
- marketplace metadata
- icons and screenshots
- privacy and security notes
- upgrade flow

Acceptance:

- Fresh install works without a preinstalled `remem`.
- Upgrade replaces the plugin-managed runtime safely.
- Existing user data is preserved.

## Testing matrix

| Scenario | Expected result |
| --- | --- |
| Empty machine, no `remem` on `PATH` | plugin installs runtime into `PLUGIN_DATA` |
| Stale system binary on `PATH` | plugin reports stale binary and uses managed runtime |
| Matching system binary exists | plugin can adopt it after explicit choice |
| Offline runtime install | plugin reports actionable offline error |
| Checksum mismatch | install fails closed |
| Manual MCP without hooks | search/save/status work |
| Hooks untrusted | automatic capture does not run, manual MCP still works |
| Hooks trusted | SessionStart injection and Stop summarization work |
| Governance mutation without dry-run/reason | rejected |
| Apps SDK app without app id | app remains local developer-mode only |

## Required verification

For implementation PRs:

```bash
python3 /Users/lifcc/.codex/skills/.system/plugin-creator/scripts/validate_plugin.py plugins/remem
cargo check
cargo test
```

For plugin runtime tests:

```bash
PLUGIN_DATA="$(mktemp -d)" PATH="/usr/bin:/bin" node plugins/remem/scripts/remem-runtime.js status
PLUGIN_DATA="$(mktemp -d)" PATH="/usr/bin:/bin" node plugins/remem/scripts/remem-mcp.js --self-test
node --test plugins/remem/scripts/remem-runtime.test.js
```

For Apps SDK tests:

```bash
cd apps/remem
npm test
npm run build
```

Then run MCP Inspector or ChatGPT Developer Mode against the local `/mcp`
endpoint.

## Key decisions

- The final plugin should be plugin-managed by default, not PATH-managed.
- Existing `remem install --target codex` remains a supported advanced path,
  but it is not the primary plugin experience.
- Automatic capture uses trusted plugin-bundled hooks where Codex supports
  them.
- Apps SDK GUI is a later layer and must not block manual memory usefulness.
- `.app.json` should not be added until a real app or connector id exists.
