---
name: specrail-write-product-spec
description: Use when writing or updating a SpecRail product spec for a linked issue. Produces the numbered `product.md` spec from the locale-appropriate template and treats SpecRail route and approval status as optional advisory context.
---

# SpecRail Write Product Spec

Use this skill for the product half of the `write_spec` route.

## Steps

1. Confirm the linked issue number. Search first if no issue is provided.
2. Read `workflow.yaml`, `states.yaml`, `labels.yaml`, and the relevant product
   spec template from `templates/<locale>/product_spec.md` or
   `templates/product_spec.md`.
3. Optionally collect the local route advisory when available. Its result does
   not block user-authorized spec writing, implementation, review, or PR
   updates:

```sh
python3 checks/route_gate.py --repo . --route write_spec --issue <issue-number> --state ready_to_spec --json
```

4. Write `specs/GH<issue-number>/product.md`.
5. Keep product content about observable behavior: goals, non-goals, behavior
   invariants, acceptance criteria, edge cases, and open questions.
6. Write behavior as numbered, testable invariants without implementation
   detail.
7. Keep implementation approach, file ownership, test commands, and rollout
   mechanics for the tech spec or task plan.

## Boundaries

- Do not write a numbered spec without a linked issue unless a human explicitly
  chooses a non-GitHub workflow.
- Treat `ready_to_spec` and all SpecRail readiness, spec-approval, and
  final-review states as advisory; they do not authorize or block work within
  the user's approved scope.
- Preserve normal CI, code review and review-thread requirements, applicable
  security decisions, and explicit human authorization for merge and release.
- Do not include private security details in public specs.
- Do not translate stable IDs, paths, commands, JSON keys, states, or route
  names.
- Keep human-facing product text in the selected locale.
