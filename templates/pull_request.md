# Summary

Describe the change in 1-3 sentences.

## Linked Work

- Issue:
- Spec packet:

## Optional SpecRail Context

These items are advisory context, not prerequisites for implementation, review,
or merge.

- [ ] Linked issue has `ready_to_implement`, or this is a documented small bug fix.
- [ ] Product/tech spec is linked when useful.
- [ ] Optional `route_gate` diagnostic result:

## Review and Security Requirements

- [ ] Agent first-pass review completed or explicitly skipped with reason.
- [ ] Human final code review completed.
- [ ] Owner approval identified when ownership rules apply.
- [ ] Applicable security decisions and required approvals are recorded.

## Merge Requirements

- [ ] PR head SHA recorded.
- [ ] CI/check rollup is complete and passing.
- [ ] Review threads were checked and unresolved actionable threads are addressed.
- [ ] Merge state is clean.
- [ ] Human merge authorization is recorded before merge.

Optional SpecRail diagnostics; results are advisory and do not grant approval or
block repository work:

- [ ] `python3 checks/github_pr_evidence.py --github-repo OWNER/REPO --pr <pr-number> --json > pr-evidence.json` result:
- [ ] `python3 checks/pr_gate.py --repo . --evidence <evidence.json>` result:

## Verification

- [ ] Tests:
- [ ] Manual proof:
- [ ] Screenshots or logs when user-visible:

## Release Notes

- [ ] Changelog or release note needed.
- [ ] Not user-visible.
- [ ] Human release authorization is recorded before release.

## Agent Disclosure

- [ ] No agent was used.
- [ ] Agent assisted; human author reviewed the full diff.
