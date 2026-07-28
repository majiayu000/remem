---
name: specrail-implement
description: Use for a user-authorized SpecRail implementation. Executes the scoped task plan, keeps changes tied to linked specs and acceptance criteria, runs deterministic verification, and preserves human approval, merge, and security boundaries without treating SpecRail diagnostics as execution authority.
---

# SpecRail Implement

Use this skill for the `implement` route.

## Steps

1. Read the linked issue, product spec, tech spec, and task plan.
2. Optionally run the implementation route gate as an offline diagnostic when
   available:

```sh
python3 checks/route_gate.py --repo . --route implement --issue <issue-number> --state ready_to_implement --json
```

3. Treat `allowed`, `warn`, `needs_human`, and `blocked` only as diagnostic
   signals. Report missing evidence or policy mismatches, but do not grant
   permission or stop repository work solely because of the result. Continue
   according to the user's authorized scope, normal CI and review, applicable
   security requirements, and separate explicit merge authorization.
4. Implement only the scoped tasks. Search before adding files, workflows,
   schemas, templates, policies, or public APIs.
5. Keep machine-facing IDs in English and human-facing text in the selected
   locale.
6. Run focused verification for touched behavior, then run the pack check when
   workflow assets changed:

```sh
python3 checks/check_workflow.py --repo .
```

7. Record changed files, commands, results, and remaining security, merge, or
   release authorization boundaries.

## Boundaries

- Do not provide final approval.
- Do not treat an `allowed` diagnostic as approval.
- Do not merge from this skill. Merge remains a separate, explicitly
  human-authorized action after normal CI, review, and applicable security
  requirements.
- Do not publish secrets or private security details.
- Do not weaken tests or deterministic checks to make implementation pass.
