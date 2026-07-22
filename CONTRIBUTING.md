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

Use this ladder to choose the smallest useful checks for the change:

| Change type | Focused checks |
| --- | --- |
| Docs only | `git diff --check` and review rendered Markdown when formatting matters. |
| CLI behavior | Run the targeted CLI parser/action test, then `cargo test <changed_surface>` for the affected command. |
| Plugin or npm wrapper | Run `node --test plugins/remem/scripts/remem-runtime.test.js plugins/remem/apps/remem/request-security.test.js plugins/remem/apps/remem/server.test.js npm/remem/scripts/install.test.js`; include `python3 scripts/ci/check_plugin_version_sync.py` when versions, runtime assets, or plugin metadata change. |
| API behavior | Run the focused API test such as `cargo test api` or `cargo test --test api_public`, then check the relevant API docs/spec note. |
| Eval changes | Run the focused eval command or test for the changed fixture/gate, then `cargo run -- eval-gates --json-out /tmp/remem-eval-gates.json` when thresholds or committed baselines are touched. |

Behavior changes need targeted regression tests that prove the new behavior
and the old failure mode before relying on broad `cargo test`.

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

### Enforcement-sensitive changes

Every PR must include exactly one machine-readable declaration:

```text
enforcement_sensitive: false
```

Set it to `true` when the changed paths or linked specs match
`workflow.yaml`'s `enforcement.sensitive_registry`. Sensitive work has no
fast path: it requires an approved Product/Tech contract, a terminal
independent review artifact bound to the final head, complete prior-finding
carry-forward, authorized resolution of actionable threads, green CI, and an
`allowed` exact-head PR gate result. Author-entered prose or a bare
`review_source` value is not review evidence.

The repository-local gates fail closed for agent workflows, but they are only
advisory. The maintainer-selected `main` ruleset may require the `check` status,
review-conversation resolution, and block force pushes and deletion without a
second approver. GitHub required status checks do not bind a check name to one
workflow or event, however, so a GitHub Actions-sourced `check` is an
accidental-bypass mitigation, not an unforgeable authorization. Non-bypass
enforcement requires a separately operated GitHub App whose expected source is
pinned by the ruleset, or an organization-level required workflow from a
protected governance repository. Repository permission and external trust-root
changes remain human-admin work.

After an enforcement-sensitive PR is merged, the repo-local closure workflow
uses the PR's pre-merge base registry/controller to audit the same-head
gate/dispatch/merge chain. A missing chain creates or reopens one durable issue
keyed by repository, PR number, final head, and violation code. GitHub write or
read-back failures block closure; a local artifact alone does not count as a
persisted follow-up. Because a merged PR can delete a repository-owned workflow
before its closed event is dispatched, this audit is compensating evidence, not
the external T6 trust root.
