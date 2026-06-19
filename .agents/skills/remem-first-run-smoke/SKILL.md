---
name: remem-first-run-smoke
description: Validate remem first-run behavior from an isolated HOME and REMEM_DATA_DIR. Use when changing install, doctor, status, encrypt, hook activation, Codex/Claude setup, plugin activation, or user-facing onboarding documentation.
---

# remem-first-run-smoke

Use this skill to test the path a new user experiences without touching the
developer's real `~/.codex`, `~/.claude`, or `~/.remem` directories.

## Rules

- Always use a temporary `HOME` and `REMEM_DATA_DIR`.
- Prefer `--dry-run` before activation commands that edit host config.
- Do not run hook activation against the real home directory during smoke tests.
- Keep stdout/stderr logs when the smoke fails; install and status failures are
  usually more useful than a later generic test failure.

## CLI Smoke

Run from the repository root:

```bash
cargo build --release
tmp_home="$(mktemp -d)"
tmp_data="$(mktemp -d)"

HOME="$tmp_home" REMEM_DATA_DIR="$tmp_data" target/release/remem install --target codex --dry-run
HOME="$tmp_home" REMEM_DATA_DIR="$tmp_data" target/release/remem install --target codex
HOME="$tmp_home" REMEM_DATA_DIR="$tmp_data" target/release/remem status
HOME="$tmp_home" REMEM_DATA_DIR="$tmp_data" target/release/remem context --host codex-cli >/tmp/remem-context.txt
HOME="$tmp_home" REMEM_DATA_DIR="$tmp_data" target/release/remem doctor || doctor_status=$?
```

Expected result:

- install writes only inside the temporary home/data directories
- status completes without requiring manual repair
- context exits successfully and writes deterministic, bounded output
- doctor may exit non-zero before any real SessionStart/Stop heartbeat exists;
  treat that as expected only when the diagnostic output is limited to the fresh
  capture-liveness warning. After a synthetic or real hook heartbeat, doctor
  should pass without that warning.

Remove the temporary directories after inspecting failures.

## Plugin Activation Smoke

For Codex plugin activation changes, first install the repo-built runtime into
the plugin-managed store, then run activation in dry-run mode against a temp
home:

```bash
cargo build --release
tmp_home="$(mktemp -d)"
tmp_data="$(mktemp -d)"

HOME="$tmp_home" REMEM_DATA_DIR="$tmp_data" \
  REMEM_BINARY="$PWD/target/release/remem" \
  node plugins/remem/scripts/remem-runtime.js install

HOME="$tmp_home" REMEM_DATA_DIR="$tmp_data" \
  node plugins/remem/scripts/activate-codex.js --dry-run
```

Only run `activate-codex.js` without `--dry-run` when the task explicitly needs
to verify written hook/config files, and still keep `HOME` pointed at the temp
directory.

## Focused Automated Checks

Run the existing install/runtime tests after smoke commands:

```bash
node --test plugins/remem/scripts/remem-runtime.test.js plugins/remem/apps/remem/server.test.js npm/remem/scripts/install.test.js
cargo test --test install_status
cargo test install::tests
```

For user-facing onboarding or memory-quality changes, also run:

```bash
cargo run -- eval-extraction --json --check-baseline
cargo run -- eval-gates --json-out /tmp/remem-eval-gates.json
```
