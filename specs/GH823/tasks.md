# Task Plan

## Linked Issue

GH-823

## Spec Packet

- Product: `product.md`
- Tech: `tech.md`

## Implementation Tasks

- [ ] `SP823-T1` Owner: maintainer with access to a real Cursor installation; Dependencies: GH-822; Done when: the tested Cursor version, sanitized sessionStart/postToolUse/stop payloads, unique model-visible context marker, foreground/background behavior, path forms, bounded tool-field sizes, and real MCP invocation probes for postToolUse/beforeMCPExecution/afterMCPExecution are recorded without inferred mappings; Verify: GH-822 evidence bundle and maintainer attestation.
- [ ] `SP823-T2` Owner: human maintainer; Dependencies: `SP823-T1`; Done when: a human reviews the real-host evidence, freezes `CURSOR_TOOL_FIELD_MAX_BYTES` and all unresolved identity/event policies, approves `product.md` and `tech.md`, and explicitly moves GH-823 to `ready_to_implement`; Verify: GitHub approval/readiness evidence at the approved spec head.
- [ ] `SP823-T3` Owner: implementation agent; Dependencies: `SP823-T2`; Done when: canonical Cursor host parsing, strict sessionStart context parsing/rendering, PII sanitization, fail-closed errors, and session-init rejection implement B-001..B-006 and B-009..B-014 without changing Claude/Codex behavior; Verify: focused unit/subprocess/DB tests, PII sentinel tests, `cargo fmt --check`, and `cargo check`.
- [ ] `SP823-T4` Owner: implementation agent; Dependencies: `SP823-T2`; Done when: postToolUse maps only verified fields, decodes bounded tool_input/tool_output before classification, and sends unknown tool names through verbatim generic capture with no diagnostic drop; Verify: B-007/B-015 boundary, generic-capture, spill, adapter, and zero-write failure tests.
- [ ] `SP823-T5` Owner: implementation agent; Dependencies: `SP823-T2` and GH-825 merged with approved Cursor transcript fixtures; Done when: Cursor stop maps verified identity/status fields and cannot invoke the Claude/Codex transcript reader, enqueue, spill, or LLM path before GH-825's reader is available; Verify: reader-spy prerequisite test plus B-008 status and persistence tests.
- [ ] `SP823-T6` Owner: verification agent; Dependencies: `SP823-T3` `SP823-T4` `SP823-T5`; Done when: every B-001..B-016 mapping passes, MCP behavior matches the approved real probe, Claude/Codex fixtures remain unchanged, and no user_email sentinel reaches any sink; Verify: focused tests, `cargo test`, `python3 checks/check_workflow.py --repo . --spec-dir specs/GH823`, and `git diff --check`.

No implementation task (`SP823-T3` or later) may start until `SP823-T2` has
fresh human approval at the exact spec head. Autonomous agents may not infer
approval from this packet, documentation, a successful local fixture, or the
existence of PR #827.

## Parallelization

- `SP823-T1` and `SP823-T2` are serialized human gates and own evidence/state,
  not repository implementation files.
- After `SP823-T2`, `SP823-T3` and `SP823-T4` may run in isolated worktrees with
  disjoint file ownership fixed by the approved tech plan.
- `SP823-T5` cannot run in parallel with GH-825's transcript reader work and
  starts only after that prerequisite is merged.
- `SP823-T6` is read-only verification after all implementation tasks finish.

## Verification

- `python3 checks/check_workflow.py --repo .`
- `python3 checks/check_workflow.py --repo . --spec-dir specs/GH823`
- `git diff --check`
- Implementation-only after the human gate: `cargo fmt --check`, `cargo check`,
  focused tests, and `cargo test`.

## Handoff Notes

- GH-822 is a real-host/manual evidence gate. It must exercise an actual MCP
  call and instrument all three candidate event surfaces; guessed mappings do
  not unblock implementation.
- GH-825 is a hard prerequisite for Cursor summarize. Until it lands, Cursor
  transcript paths must not reach the existing Claude/Codex raw parser.
- PR #827 is a draft specification change only. It does not authorize runtime
  implementation, installation, thread resolution, or merge.
