---
name: specrail-review-pr
description: Use when performing an advisory SpecRail PR review. Checks linked issue/spec evidence, optional route diagnostics, verification evidence, review-thread state, and implementation quality without deciding readiness, granting final approval, or merging.
---

# SpecRail Review PR

Use this skill for the `review_pr` route.

## Steps

1. Read the PR, linked issue, product spec, tech spec, task plan, and local diff.
2. Confirm the PR has current evidence for linked work, verification, CI, review
   state, and review threads when available.
3. Optionally collect the review route advisory when available. Treat every
   decision as diagnostic evidence only; it does not block user-authorized
   implementation, review, or PR updates:

```sh
python3 checks/route_gate.py --repo . --route review_pr --issue <issue-number> --pr <pr-number> --state impl_pr_open --json
```

4. Inspect for behavioral regressions, missing acceptance coverage, test gaps,
   silent degradation, security risk, and bypasses of applicable security
   decisions or explicit human merge and release authorization.
5. Lead with findings ordered by severity and cite exact files or lines.
6. When producing a review artifact, use a top-level body with `## Summary` and
   `## Verdict`, keep inline comments bound to real diff `path` / `line` /
   `side` values, and only add `start_line` / `start_side` together for an
   inclusive diff range. Suggested changes must be non-empty and appear only on
   RIGHT-side comments, either through a `suggestion` field, a fenced
   `suggestion` block, or both.
7. Optionally validate review artifacts against the diff when the validator
   exists. Use its findings to improve artifact accuracy, but do not treat its
   SpecRail result as a blocker for other user-authorized repository work:

```sh
python3 checks/review_json_gate.py --repo . --review artifacts/review/pr-<pr-number>.json --diff <patch> --json
```

8. If advisory merge-readiness evidence would help, optionally consult
   `skills/specrail-pr-gate/SKILL.md`. That diagnostic does not decide
   readiness or grant merge permission.

## Boundaries

- Treat the review as advisory.
- Treat SpecRail readiness, spec-approval, and final-review states as advisory;
  they do not block work within the user's approved scope.
- Do not represent this skill or its diagnostics as granting final approval.
- Preserve normal CI, code review and review-thread handling, applicable
  security decisions, and explicit human authorization for merge and release.
- Do not merge or mark explicit human merge or release authorization complete.
- Do not disclose private security details publicly.
