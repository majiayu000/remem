## PR Type

- [ ] Spec only
- [ ] Implementation
- [ ] Bugfix
- [ ] Release/docs/process

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
- [ ] If touching `src/api/**`, updated `docs/specs/SPEC-web-api.md` or wrote `API contract docs: not needed`

## Test Plan

- [ ] Tests pass
- [ ] Tested manually
