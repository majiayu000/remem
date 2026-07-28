# Agent Usage

## Optional SpecRail Reference

This repository retains SpecRail as an optional, offline workflow reference.
Its skills and checks can help with issue triage, spec writing, task planning,
implementation review, CI diagnosis, and release-note drafting, but they are
not connected to CI and do not authorize or block repository actions.

When the user requests SpecRail, start with
`skills/specrail-workflow/SKILL.md`. After the route is known, load exactly one
focused skill from `skills/specrail-*/SKILL.md`; do not load every SpecRail
skill up front.

## Artifacts

- Workflow config: `workflow.yaml`
- State graph: `states.yaml`
- Label groups: `labels.yaml`
- Repo-local skills: `skills/specrail-*/SKILL.md`
- Skill lock: `skills-lock.json`
- SpecRail issue packet: `specs/GH<issue-number>/product.md`,
  `specs/GH<issue-number>/tech.md`, and
  `specs/GH<issue-number>/tasks.md`
- Existing remem contracts and historical implementation specs:
  `docs/specs/`

Do not replace the existing `docs/specs/` index with SpecRail packets. Use
`docs/specs/` for current remem contracts and history, and use
`specs/GH<issue-number>/` for new issue-first SpecRail work.

## Authority and Safety

Agents may draft, implement, diagnose, and review. Normal CI and code review
remain required by project policy. Agents must not merge, force-push, publish
private security details, change repository permissions, make security
decisions, or release without explicit human authorization.

SpecRail readiness, spec-approval, final-review, PR-gate, and closure results
are not repository gates. The retained checks may be run for optional offline
diagnostics:

```sh
python3 checks/route_gate.py --repo . --route <route> --issue <issue-number> --state <state> --json
```

For changes to the retained offline workflow pack, its consistency checker is
also available:

```sh
python3 checks/check_workflow.py --repo .
```

Neither command replaces CI, human review, security review, merge
authorization, or release authorization.
