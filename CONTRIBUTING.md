# Contributing to remem

Thanks for your interest in contributing!

## Development Setup

```bash
git clone https://github.com/majiayu000/remem.git
cd remem
cargo build
cargo test
```

## Local Checks

For code changes, run the narrowest focused tests first, then run the broader
gate before submission when practical:

```bash
cargo fmt --check
cargo check
cargo test
```

Pull request CI also runs plugin/runtime and release-safety gates:

```bash
python3 scripts/ci/check_plugin_version_sync.py
node --test plugins/remem/scripts/remem-runtime.test.js plugins/remem/apps/remem/server.test.js npm/remem/scripts/install.test.js
python3 scripts/ci/check_version_bump.py <base-sha> HEAD
cargo clippy -- -D warnings
cargo run -- eval-extraction --json --check-baseline
cargo run -- eval-gates --json-out /tmp/remem-eval-gates.json
```

Use repo-local skills under `.agents/skills/` for fragile maintenance flows
such as plugin version synchronization and first-run smoke validation.

## Guidelines

- Follow existing code style
- Add tests for new features
- Commit messages: `<type>: <description>` (feat/fix/refactor/docs/test/chore)

## Pull Requests

1. Fork the repo and create your branch from `main`
2. Ensure tests pass
3. Submit a PR with a clear description
