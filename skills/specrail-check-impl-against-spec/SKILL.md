---
name: specrail-check-impl-against-spec
description: Use when performing an advisory comparison of a SpecRail implementation, diff, or PR against its linked issue, product spec, technical spec, and task plan. Reports acceptance coverage, mismatches, omitted tasks, extra scope, and verification gaps without deciding readiness, approving, or merging.
---

# SpecRail Check Implementation Against Spec

Use this skill when the question is whether implementation matches the spec.

## Steps

1. Read the linked issue, `product.md`, `tech.md`, `tasks.md`, and the diff or
   PR under review.
2. Map every acceptance criterion and task ID to implementation evidence,
   verification evidence, or a missing item.
3. Identify extra behavior not requested by the spec.
4. Check that stable IDs, paths, JSON keys, states, and commands remain in
   English.
5. Record any available SpecRail route, PR, review, readiness, spec-approval,
   or final-review status as optional advisory evidence. It does not block
   user-authorized implementation, review, or PR updates.
6. Report results as:
   - covered
   - missing
   - mismatched
   - extra scope
   - needs human decision
7. Recommend the smallest corrective action for each gap.

## Boundaries

- Treat this comparison and all SpecRail status findings as advisory; they do
  not authorize or block work within the user's approved scope.
- Do not treat partial coverage as approval or weaken normal verification.
- Do not rewrite the spec to match an implementation unless the user asks for a
  spec revision.
- Preserve normal CI, code review and review-thread requirements, applicable
  security decisions, and explicit human authorization for merge and release.
- Do not merge or provide final approval from this skill.
