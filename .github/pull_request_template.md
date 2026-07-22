## PR Type

- [ ] Spec only
- [ ] Implementation
- [ ] Bugfix
- [ ] Release/docs/process

Tier: standard
enforcement_sensitive: false

`enforcement_sensitive` is machine-checked against `workflow.yaml`. Change it
to `true` when the diff or linked spec matches the sensitive registry. There is
no fast path for sensitive work.

## What

Brief description of changes.

## Why

Why is this change needed?

## Issue Links

- Refs #
- Closes #

Spec-only PRs must use `Refs #...`, not `Closes` / `Fixes` / `Resolves`,
unless the linked issue is explicitly only about writing the spec. Runtime or
user-visible work must close an implementation issue, not a spec issue.

## Spec Lifecycle

- [ ] If adding/updating `docs/specs/<id>/`, updated `docs/specs/README.md`
- [ ] If this is spec-only, linked or created implementation issue(s)
- [ ] If this is implementation, updated relevant spec, README, API docs, or wrote why not needed
- [ ] If touching `src/api/**`, updated `docs/specs/SPEC-web-api.md` or wrote an explicit API docs waiver with rationale

## Test Plan

- [ ] Tests pass
- [ ] Tested manually

## Review Gate

- Final head SHA:
- Independent review artifact or run:
- Review completed at:
- Prior findings carried forward and resolved/obsolete with evidence:
- Actionable review threads resolved by an authorized reviewer or maintainer:

These fields summarize evidence; `checks/pr_gate.py` and the underlying review
artifact remain authoritative. A checked box or prose-only claim is not proof.

## Merge Gate

- [ ] Required `check` is green on the final head
- [ ] Exact-head PR gate decision is `allowed`
- [ ] Merge authorization is recorded
- [ ] External-App/org-required-workflow trust root is active, or the advisory-only gap is explicit
