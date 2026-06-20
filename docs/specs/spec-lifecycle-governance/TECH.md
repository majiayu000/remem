# Spec Lifecycle Governance Technical Spec

Status: Current contract
Date: 2026-06-21

Tracking:
- Process implementation: #592

## Existing Implementation Facts

- GitHub issue templates live under `.github/ISSUE_TEMPLATE/`.
- The PR template is `.github/pull_request_template.md`.
- CI runs `.github/workflows/ci.yml` for pull requests.
- Existing CI scripts live under `scripts/ci/`.
- `docs/specs/README.md` is the source of truth for current vs historical
  specs.

## Design

Add a lightweight CI script:

```text
scripts/ci/check_spec_lifecycle.py
```

The script reads:

- base and head revisions from CLI args;
- changed files from `git diff --name-status <base> <head>`;
- PR title/body from `GITHUB_PR_TITLE` and `GITHUB_PR_BODY`, with a fallback to
  `GITHUB_EVENT_PATH` when running in GitHub Actions.

It does not call the GitHub API. This keeps the guard deterministic, fast, and
safe for forks.

## PR Body Markers

The guard recognizes checked PR type boxes in `.github/pull_request_template.md`:

```md
- [x] Spec only
- [x] Implementation
- [x] Bugfix
- [x] Release/docs/process
```

Unchecked boxes do not count. Matching is case-insensitive for `x`.

The guard recognizes these issue link patterns:

```text
Refs #123
Closes #123
Fixes #123
Resolves #123
```

GitHub auto-close keywords are forbidden for checked spec-only PRs. They remain
valid for runtime and bugfix PRs.

## Checks

### Spec-Only PR Closing Guard

If `Spec only` is checked:

- fail if PR body contains `Closes #`, `Fixes #`, or `Resolves #`;
- fail if PR body does not contain `Refs #`.

### Current Spec Directory Guard

If a PR adds `docs/specs/<id>/PRODUCT.md` or `docs/specs/<id>/TECH.md`, where
`<id>` is not `refactor-steps`, then:

- the matching `PRODUCT.md` and `TECH.md` must both exist at `HEAD`;
- `docs/specs/README.md` must be changed in the PR.

This keeps new current specs indexed and prevents orphan TECH-only or
PRODUCT-only specs.

### Runtime Traceability Guard

If a PR changes `src/**`, then:

- the PR body must contain an auto-close issue link, or
- the PR body must include `No issue: ...`.
- the PR must not be checked as `Spec only`.

This keeps runtime changes traceable while allowing small urgent fixes with an
explicit explanation.

### API Contract Guard

If a PR changes `src/api/**`, then:

- `docs/specs/SPEC-web-api.md` must be changed, or
- the PR body must include `API contract docs: not needed`.

This forces every API change to make a documentation decision. It does not
attempt to infer whether the API shape actually changed.

## Failure Messages

Every failure should include:

- what failed;
- why it matters;
- the smallest likely fix.

Example:

```text
Spec-only PRs must use Refs, not Closes/Fixes/Resolves. Replace "Closes #123"
with "Refs #123" and create/link implementation issue(s).
```

## CI Integration

Add the guard before expensive Rust steps:

```yaml
- name: Check spec lifecycle
  if: github.event_name == 'pull_request'
  env:
    GITHUB_PR_TITLE: ${{ github.event.pull_request.title }}
    GITHUB_PR_BODY: ${{ github.event.pull_request.body }}
  run: python3 scripts/ci/check_spec_lifecycle.py "${{ github.event.pull_request.base.sha }}" HEAD
```

## Local Usage

```bash
python3 scripts/ci/check_spec_lifecycle.py origin/main HEAD
python3 scripts/ci/check_spec_lifecycle.py origin/main WORKTREE
python3 scripts/ci/check_spec_lifecycle.py --self-test
```

For local testing, PR body can be passed through env:

```bash
GITHUB_PR_BODY="$(cat /tmp/pr-body.md)" \
python3 scripts/ci/check_spec_lifecycle.py origin/main HEAD
```

## Test Cases

`--self-test` covers:

- spec-only PR with `Refs #123` passes;
- spec-only PR with `Closes #123` fails;
- checked implementation with `src/**` and `Closes #123` passes;
- checked implementation with `src/**` and no issue link fails;
- checked bugfix with `src/**` and no issue link fails;
- checked release/docs/process with `src/**` and no issue link fails;
- checked spec-only with `src/**` fails;
- new current spec missing `docs/specs/README.md` fails;
- new current spec with PRODUCT, TECH, and README update passes;
- API source change without docs update or explicit marker fails;
- API source change with `API contract docs: not needed` passes.

## Alternatives Considered

### Option A: Documentation Only

**Pros**

- No CI complexity.
- No false positives.

**Cons**

- The observed failure mode can repeat.
- Reviewers and agents must remember a non-obvious lifecycle rule every time.

**Decision**: Rejected. Documentation remains necessary, but not sufficient.

### Option B: GitHub Projects / Labels Only

**Pros**

- Better human triage and dashboards.
- Can represent epic/spec/implementation state.

**Cons**

- Does not prevent a spec-only PR from closing an issue.
- Requires more manual upkeep.

**Decision**: Deferred. Useful later, but not enough for enforcement.

### Option C: CI PR Body And Diff Guard (Recommended)

**Pros**

- Directly blocks the known failure mode.
- Fast, local, and independent of GitHub API permissions.
- Works with current templates and normal PR review.

**Cons**

- Regex-based checks can produce false positives.
- Contributors must update PR body checkboxes accurately.

**Decision**: Chosen for this implementation.

## Risks And Mitigations

| Risk | Mitigation |
|---|---|
| False positives on legitimate PRs | Provide explicit escape hatches and actionable messages. |
| PR body omitted or stale | Fail only for conditions that need body intent; templates make intent visible. |
| CI expression quoting issues | Pass title/body through env and parse event JSON as fallback. |
| Guard drifts from process docs | Keep this spec, issue templates, and PR template in the same PR. |

## Acceptance Criteria

- `scripts/ci/check_spec_lifecycle.py --self-test` passes.
- `python3 scripts/ci/check_spec_lifecycle.py origin/main HEAD` passes for this
  PR when `GITHUB_PR_BODY` matches the PR template.
- CI invokes the guard on pull requests.
- PR and issue templates reflect the lifecycle contract.
