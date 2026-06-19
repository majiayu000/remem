---
name: remem-plugin-version-sync
description: Keep remem binary, Cargo, Codex plugin, release manifest, and npm wrapper versions synchronized. Use when changing package.version, release metadata, plugin runtime behavior, npm wrapper behavior, or when CI/check_plugin_version_sync.py or check_version_bump.py fails.
---

# remem-plugin-version-sync

Use this skill for changes that affect the shipped `remem` binary, Codex plugin
runtime, or npm wrapper. The goal is to keep all release surfaces consistent and
avoid a plugin that resolves a stale or incompatible runtime.

## Files

Keep these synchronized when the binary version changes:

- `Cargo.toml`
- `Cargo.lock`
- `plugins/remem/.codex-plugin/plugin.json`
- `plugins/remem/runtimes/remem-releases.json`
- `npm/remem/package.json`

`scripts/ci/check_plugin_version_sync.py` requires the Rust package, plugin
manifest, release manifest, and npm wrapper package versions to match. A binary
version bump must update the npm package version even when npm wrapper behavior
is unchanged.

Read these before editing plugin runtime behavior:

- `plugins/remem/README.md`
- `plugins/remem/skills/remem/SKILL.md`
- `plugins/remem/scripts/remem-runtime.js`
- `plugins/remem/scripts/remem-runtime.test.js`
- `npm/remem/scripts/install.js`
- `npm/remem/scripts/install.test.js`

## Workflow

1. Run `git status --short` and identify whether the PR changes binary-impacting
   files: `src/`, `migrations/`, or `Cargo.lock`.
2. If binary-impacting files changed, bump `Cargo.toml` `package.version`.
3. Regenerate or update `Cargo.lock` so the `remem-ai` package version matches.
4. Update `plugins/remem/.codex-plugin/plugin.json` to the same version.
5. Update `plugins/remem/runtimes/remem-releases.json` so it contains exactly the
   same version key and a `base_url` ending in `/releases/download/v<version>`.
6. Update `npm/remem/package.json` to the same version. For npm-only metadata or
   wrapper behavior changes, update this file without bumping the binary only
   when `check_plugin_version_sync.py` still passes.
7. Keep runtime resolution explicit: `REMEM_BINARY` and plugin-managed runtime
   paths are valid; do not silently adopt `remem` from `PATH`.

## Verification

Run these focused checks after version or plugin-runtime changes:

```bash
python3 scripts/ci/check_plugin_version_sync.py
node --test plugins/remem/scripts/remem-runtime.test.js plugins/remem/apps/remem/server.test.js npm/remem/scripts/install.test.js
python3 scripts/ci/check_version_bump.py origin/main HEAD
```

For Rust runtime changes, also run:

```bash
cargo fmt --check
cargo check
cargo test
```

## Failure Handling

- If `check_plugin_version_sync.py` fails, update all listed version files
  together. Do not bypass the check with environment flags.
- If `check_version_bump.py` fails, either bump `Cargo.toml` or prove the changed
  file is not binary-impacting and update the gate separately.
- Use `REMEM_ALLOW_VERSION_MISMATCH=1` only for explicit local debugging, never
  as a PR validation substitute.
