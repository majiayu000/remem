---
name: specrail-release-note
description: Use when drafting a SpecRail release note after a linked PR has merged. Summarizes user-visible changes, verification, linked issues, risks, and rollout notes while treating SpecRail diagnostics as advisory and preserving explicit release and security authority.
---

# SpecRail Release Note

Use this skill for the `draft_release_note` route.

## Steps

1. Confirm the PR is merged and identify the linked issue, commits, specs, and
   verification evidence.
2. Optionally collect the release-note route advisory when available. Its
   result does not block user-authorized drafting or PR updates and does not
   authorize publication:

```sh
python3 checks/route_gate.py --repo . --route draft_release_note --issue <issue-number> --pr <pr-number> --state merged --json
```

3. Draft a concise release note in the selected locale.
4. Include user-visible change, linked work, verification, migration or rollback
   notes, and any known limitations.
5. Keep stable machine-facing IDs, paths, commands, and JSON keys in English.

## Boundaries

- Treat SpecRail readiness, spec-approval, and final-review states as advisory;
  they do not block user-authorized release-note drafting.
- Preserve normal CI, code review and review-thread requirements, applicable
  security decisions, and explicit human authorization for merge and release.
- Do not publish a release.
- Do not mark explicit human release authorization complete.
- Do not include private security details in public notes.
- Do not claim closure for unverified issues or PRs.
