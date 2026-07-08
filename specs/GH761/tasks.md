# GH761 Task Plan

## Linked Issue

GH-761

## Spec Packet

- Product: `product.md`
- Tech: `tech.md`

## Spec-Only Handoff

- [ ] `SP761-T0` Owner: SpecRail coordinator; Dependencies: spec approval; Done when: after GH-761 moves to `ready_to_implement`, replace this placeholder with implementation tasks for hook integrity evaluator, Claude SessionStart warning, hook-only repair, doctor/docs, and verification; Verify: `python3 checks/route_gate.py --repo . --route implement --issue 761 --state ready_to_implement --json`.

## Verification

- `python3 checks/check_workflow.py --repo .`
- `python3 checks/check_workflow.py --repo . --spec-dir specs/GH761`

## Handoff Notes

- This write-spec PR does not authorize runtime implementation by itself.
- Do not start GH-761 implementation from this task file until the issue is moved through the SpecRail implementation gate.
- Implementation PRs may use `Refs #761`; only a final implementation PR with warning, repair, doctor/docs, tests, and full verification may use `Closes #761`.
