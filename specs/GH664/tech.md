# Tech Spec

## Linked Issue

GH-664

## Product Spec

Link to `product.md`.

## Codebase Context

| Area | Files | Current behavior | Why relevant |
| --- | --- | --- | --- |
| Agent instructions | `AGENTS.md` | Requires `git status`, current docs, search-before-create, and existing `docs/specs/` routing. | Needs a local SpecRail trigger without weakening remem-specific rules. |
| Spec index | `docs/specs/README.md` | Treats `docs/specs/` as current contracts plus historical implementation evidence. | Needs to distinguish existing remem specs from new SpecRail issue packets. |
| Repo-local skills | `skills/specrail-*/SKILL.md`, `skills-lock.json` | No repo-local SpecRail skills exist. | Required for reproducible route startup and focused skill routing. |
| Workflow config | `workflow.yaml`, `states.yaml`, `labels.yaml` | No SpecRail workflow pack is declared. | Route gates need deterministic state, label, action, and artifact config. |
| Checks/templates | `checks/`, `templates/`, `schemas/`, `review/`, `policies/`, `integrations/`, `tools/` | Only global SpecRail assets exist outside this repository. | Repo-local use needs local checks, templates, schema files, and usage references. |

## 设计方案

Add the SpecRail workflow pack as repo-local governance assets:

- Copy focused SpecRail skills under `skills/specrail-*/SKILL.md`.
- Add `skills-lock.json` with SHA-256 hashes for the repo-local skills.
- Add workflow config at `workflow.yaml`, `states.yaml`, and `labels.yaml`.
- Add route and review checks under `checks/`, plus schemas, templates, policy docs, review docs, optional threads integration, and the local skill installer script.
- Add `AGENT_USAGE.md` with remem-specific usage boundaries.
- Update `AGENTS.md` so agents start with `skills/specrail-workflow/SKILL.md`, then load exactly one focused route skill.
- Update `docs/specs/README.md` to define `docs/specs/` versus `specs/GH...` responsibilities.

## Product-to-Test Mapping

| Product invariant | Implementation area | Verification |
| --- | --- | --- |
| P1 | `AGENTS.md`, `skills/specrail-workflow/SKILL.md`, `skills-lock.json` | `python3 checks/check_workflow.py --repo .` |
| P2 | `AGENTS.md`, `AGENT_USAGE.md`, focused skill docs | Manual diff review plus `python3 checks/check_workflow.py --repo .` |
| P3 | `workflow.yaml`, `states.yaml`, `labels.yaml`, `checks/`, `templates/`, `schemas/` | `python3 checks/check_workflow.py --repo .`; route gate smoke commands |
| P4 | `AGENTS.md`, `AGENT_USAGE.md`, `docs/specs/README.md`, `workflow.yaml` | Diff review and `python3 checks/route_gate.py --repo . --route write_spec --issue 664 --state ready_to_spec --json` |
| P5 | `workflow.yaml`, `skills/specrail-workflow/SKILL.md`, `AGENT_USAGE.md` | Route gate JSON includes blocked actions and human gates |
| P6 | File scope excludes runtime code | `git diff --name-only` review; no Rust/JS runtime files touched |

## 数据流

SpecRail checks read repository files only:

1. Agent reads `skills/specrail-workflow/SKILL.md`.
2. Agent reads `AGENT_USAGE.md`, `workflow.yaml`, `states.yaml`, `labels.yaml`, and the route template.
3. Agent runs `checks/route_gate.py` with issue or PR evidence.
4. Route checks load config via `checks/specrail_lib.py` and emit JSON decisions.
5. Spec or review artifacts are written under `specs/GH<issue-number>/` or `artifacts/` according to `workflow.yaml`.

GitHub evidence collectors remain read-only. This PR does not add automatic writes to GitHub beyond normal issue/PR work initiated by the agent with explicit user scope.

## 备选方案

- Only add `skills-lock.json` and `skills/specrail-workflow/SKILL.md`: rejected because route gates, templates, and focused skills would still depend on global machine state.
- Put SpecRail packets under `docs/specs/GH...`: rejected because the standard SpecRail skills and workflow config use `specs/GH...`; `docs/specs/` already has a separate remem contract/history role.
- Install global SpecRail skills into `$HOME`: rejected because repo adoption should be reproducible from checkout without mutating user home config.

## 风险

- Security: No secrets or permissions are added. Checks must remain local/read-only unless a human explicitly triggers GitHub actions.
- Compatibility: New root-level workflow files may be unexpected for contributors; `AGENT_USAGE.md` and `AGENTS.md` document the boundary.
- Performance: No runtime path is affected.
- Maintenance: Copied skills can drift from upstream SpecRail; `skills-lock.json` and `checks/check_workflow.py` make drift visible.

## 测试计划

- [ ] Workflow pack validation: `python3 checks/check_workflow.py --repo .`
- [ ] Write-spec route smoke: `python3 checks/route_gate.py --repo . --route write_spec --issue 664 --state ready_to_spec --json`
- [ ] Implement route smoke after spec packet exists: `python3 checks/route_gate.py --repo . --route implement --issue 664 --state ready_to_implement --json`
- [ ] Whitespace validation: `git diff --check`

## 回滚方案

Revert the PR that adds the SpecRail workflow assets. This removes repo-local SpecRail adoption without affecting remem runtime state, database migrations, installed hooks, or release artifacts.
