# Spec Lifecycle Governance Product Spec

Status: Current contract
Date: 2026-06-21

Tracking:
- Process implementation: #592

## Problem

remem uses product and technical specs for substantial behavior, API, DB, hook,
plugin, and architecture changes. The repository already documents that current
specs live under `docs/specs/`, and that historical specs must not be treated as
raw backlog without verification.

The remaining gap is issue and PR lifecycle. A spec-only PR can accidentally use
`Closes #...` and close a feature issue before the runtime behavior is
implemented. When that happens, the repository appears done even though the
actual user-visible capability still needs implementation work, tests, smoke
checks, and docs updates.

## Goal

Make spec lifecycle explicit and mechanically checked:

1. A user-visible capability has an epic or feature issue.
2. A spec PR documents the product and technical contract and uses `Refs #...`.
3. Implementation issue(s) are created or linked after the spec is accepted.
4. Implementation PRs close implementation issues, not spec-only issues.
5. The epic closes only after all acceptance criteria are implemented and
   verified.

## Non-Goals

- Do not require specs for every small bug fix.
- Do not require GitHub Projects or a hosted project-management workflow.
- Do not block implementation PRs from closing implementation issues.
- Do not infer completion from old historical specs.
- Do not enforce semantic correctness of every spec sentence in CI.

## Product Contract

Use three issue types for substantial work:

| Type | Purpose | Closed by |
|---|---|---|
| Epic / Capability | User-visible capability or multi-PR effort. | Human or final implementation PR after acceptance criteria are met. |
| Spec Work | Product/technical contract only. | Spec PR only, if the issue is explicitly spec-only. |
| Implementation Task | Executable code/docs/tests/smoke slice. | Implementation PR with verification. |

Use four PR types:

| PR Type | Allowed issue wording | Required handoff |
|---|---|---|
| Spec only | `Refs #...` | Link or create implementation issue(s). |
| Implementation | `Closes #implementation_issue` | Update specs/docs/tests as needed. |
| Bugfix | `Closes #bug_issue` or `No issue: ...` | Focused regression test unless impractical. |
| Release/docs/process | May close a process/docs issue | No runtime claims unless tested. |

Spec-only PRs must not close user-visible feature or epic issues. They may close
only issues whose entire scope is writing or updating the spec.

## Workflow

```mermaid
flowchart LR
    A[Epic or Feature Issue] --> B[Spec Issue, if needed]
    B --> C[Spec PR: Refs epic/spec issue]
    C --> D[Implementation Issue(s)]
    D --> E[Implementation PR: Closes implementation issue]
    E --> F[Tests, docs, smoke, evals]
    F --> G[Close Epic after acceptance]
```

## User-Facing Behavior

Contributors opening a PR see checklist fields that force them to choose whether
the PR is spec-only, implementation, bugfix, or release/docs/process.

For spec-only PRs, CI rejects accidental `Closes`, `Fixes`, or `Resolves`
wording and requires at least one `Refs #...` link. This keeps user-visible
capability issues open until implementation exists.

For implementation PRs that touch runtime code, CI requires an implementation
issue link or a written `No issue: ...` explanation. This keeps code changes
traceable without making tiny urgent fixes impossible.

For API changes, CI requires either an update to `docs/specs/SPEC-web-api.md` or
an explicit `API contract docs: not needed` statement in the PR body.

## Success Metrics

| Metric | Current | Target | Measurement |
|---|---|---|---|
| Spec-only PRs accidentally closing feature issues | Observed once | 0 after this guard lands | CI failures and PR review |
| New current spec directories missing specs index entry | Possible | 0 | CI lifecycle guard |
| Runtime implementation PRs without issue traceability | Possible | Rare and explicit | PR body check |
| API changes missing contract decision | Manual review only | Explicit update or explanation | CI lifecycle guard |

## Risks And Mitigations

| Risk | Severity | Likelihood | Mitigation |
|---|---|---|---|
| CI guard blocks legitimate PRs | Medium | Medium | Allow explicit `No issue: ...` and `API contract docs: not needed` escape hatches. |
| Contributors check the wrong PR type | Low | Medium | Keep checklist simple and fail with actionable messages. |
| Spec issue and epic issue become bureaucratic for small fixes | Medium | Low | Only require specs for substantial behavior/API/DB/hook/plugin changes. |
| Historical specs are mistaken for active backlog | Medium | Medium | Keep `docs/specs/README.md` as the source of truth for status. |
| Regex checks miss creative closing wording | Low | Medium | Guard common GitHub auto-close keywords and rely on review for unusual phrasing. |

## Acceptance Criteria

- PR template includes PR type, issue link, spec lifecycle, and API contract
  checklist fields.
- Issue templates exist for epic, spec, and implementation work.
- `docs/specs/README.md` documents the lifecycle and points to this spec.
- CI runs a spec lifecycle guard on pull requests.
- The guard can run locally with:

```bash
python3 scripts/ci/check_spec_lifecycle.py <base> HEAD
python3 scripts/ci/check_spec_lifecycle.py --self-test
```

- The guard prevents spec-only PRs from using GitHub auto-close keywords.
- The guard requires new current spec directories to update
  `docs/specs/README.md`.

## Open Questions

1. Should the guard eventually require every epic to list child implementation
   issues before a spec PR can merge?
   Recommendation: no for now; GitHub API lookups would make CI slower and more
   brittle.
2. Should process/doc-only PRs require version bumps?
   Recommendation: no; keep version rules in `check_version_bump.py`.
3. Should stale spec status be audited on a schedule?
   Recommendation: yes later, but not part of this PR.
