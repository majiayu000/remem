# Task Plan

## Linked Issue

GH-664

## Spec Packet

- Product: `product.md`
- Tech: `tech.md`

## 实现任务

- [ ] `SP664-T1` Owner: agent; Dependencies: none; Done when: repo-local `skills/specrail-*/SKILL.md` files and `skills-lock.json` exist with matching hashes; Verify: `python3 checks/check_workflow.py --repo .`.
- [ ] `SP664-T2` Owner: agent; Dependencies: `SP664-T1`; Done when: workflow config, states, labels, checks, templates, schemas, policies, review docs, optional threads integration, and local installer script are present; Verify: `python3 checks/check_workflow.py --repo .`.
- [ ] `SP664-T3` Owner: agent; Dependencies: `SP664-T1`; Done when: `AGENTS.md` and `AGENT_USAGE.md` document SpecRail startup, focused skill routing, artifact boundaries, and human gates; Verify: `rg -n "SpecRail|specrail-workflow|specs/GH" AGENTS.md AGENT_USAGE.md`.
- [ ] `SP664-T4` Owner: agent; Dependencies: `SP664-T3`; Done when: `docs/specs/README.md` explains the boundary between existing remem specs and new SpecRail packets; Verify: `rg -n "SpecRail Issue Packets|specs/GH" docs/specs/README.md`.
- [ ] `SP664-T5` Owner: agent; Dependencies: `SP664-T1` `SP664-T2`; Done when: route gates produce deterministic JSON for representative write-spec and implement routes; Verify: `python3 checks/route_gate.py --repo . --route write_spec --issue 664 --state ready_to_spec --json` and `python3 checks/route_gate.py --repo . --route implement --issue 664 --state ready_to_implement --json`.
- [ ] `SP664-T6` Owner: agent; Dependencies: `SP664-T1` `SP664-T2` `SP664-T3` `SP664-T4` `SP664-T5`; Done when: the PR contains only workflow/spec/governance files and no remem runtime code changes; Verify: `git diff --name-only origin/main...HEAD`.

## 并行拆分

No parallel writable lanes are needed. A reviewer may inspect the workflow assets read-only while the implementation lane updates docs and verification evidence.

## 验证

- `python3 checks/check_workflow.py --repo .`
- `python3 checks/route_gate.py --repo . --route write_spec --issue 664 --state ready_to_spec --json`
- `python3 checks/route_gate.py --repo . --route implement --issue 664 --state ready_to_implement --json`
- `rg -n "SpecRail|specrail-workflow|specs/GH" AGENTS.md AGENT_USAGE.md docs/specs/README.md`
- `git diff --check`

## Handoff Notes

This task bootstraps SpecRail inside remem. Future issue-backed work should start from `skills/specrail-workflow/SKILL.md`, then load exactly one focused route skill and keep stable route IDs, states, artifact IDs, JSON keys, commands, and file paths in English.
