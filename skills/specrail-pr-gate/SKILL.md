---
name: specrail-pr-gate
description: Use when adding an advisory SpecRail diagnostic to a PR readiness evaluation. Collects read-only PR evidence, runs the offline diagnostic, checks linked work, current head SHA, CI, review decision, review threads, merge state, and human merge authorization without deciding readiness or merging.
---

# SpecRail PR Gate

Use this skill to add offline SpecRail diagnostic evidence to a PR readiness
evaluation. It does not decide whether the PR is merge-ready.

## Steps

1. Collect current PR evidence. Prefer the read-only adapter when available:

```sh
python3 checks/github_pr_evidence.py --github-repo <owner/repo> --pr <pr-number> --json > <evidence.json>
```

2. Run the offline diagnostic:

```sh
python3 checks/pr_gate.py --repo . --evidence <evidence.json> --json
```

3. Confirm evidence includes linked issue, current PR head SHA, CI/check rollup,
   review decision, review-thread resolution, merge state, and human merge
   authorization.
4. Interpret decisions precisely:
   - `allowed`: the diagnostic found no unmet local SpecRail rule; this is not
     approval or permission to merge.
   - `warn`: the diagnostic found advisory concerns to report.
   - `needs_human`: the diagnostic found missing evidence or human input to
     report.
   - `blocked`: the diagnostic found a local SpecRail policy mismatch to
     report; it is not a repository-level mechanical blocker.
   None of these values grants execution or merge permission or mechanically
   stops repository work. Readiness and merge are governed separately by
   current normal CI, code review and review threads, applicable security
   requirements, merge state, and explicit human merge authorization.
5. Report the evidence file path, decision, diagnostic findings, and stale or
   missing data.

## Boundaries

- Do not merge from this skill.
- Do not present `allowed` as final approval or any other decision as
  repository-level execution authority.
- Do not treat green CI alone as merge readiness.
- Do not ignore unresolved review threads.
- Do not replace maintainer final review or human merge authorization.
