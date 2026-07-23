# Task Plan

## Linked Issue

GH-823

## Spec Packet

- Product: `product.md`
- Tech: `tech.md`

## Implementation Tasks

- [ ] `SP823-T1` Owner: maintainer with access to a real Cursor installation; Dependencies: GH-822; Done when: PR #914 exact head `c0802c42c3fc22770aecb0b7b2eec88f117f795c` is human-reviewed for Cursor 3.12.17 equal event-local session/conversation identity, distinct subagent identity, null-path behavior, generic/MCP payload types, Read failure `tool_use_id`, manual preCompact, completed/aborted numeric loop 0, conditional token fields, and the split postToolUse-proven/sessionStart-blocked marker result; remaining Write/Edit/Delete/failed-Shell, background/multi-root, Windows/UNC, numeric limits, status:error, nonzero/missing/null loop, and replay stability are evidenced or explicitly kept fail-closed; Verify: PR #914 bundle, follow-up evidence, secret scan, and maintainer attestation.
- [ ] `SP823-T2` Owner: human maintainer; Dependencies: `SP823-T1`; Done when: a human adopts the exact-head evidence, freezes the proposed `CURSOR_HOOK_STDIN_MAX_BYTES = 1_048_576` independently from `CURSOR_TOOL_FIELD_MAX_BYTES`, capability-specific context support/limits, cross-event identity equality, the accepted Stop status set, loop type/missing/null normalization and canonical `(session_id,generation_id,loop_count)` Stop key, failed-tool `tool_use_id`/precedence policy, and exactly one B-016 MCP capture ownership branch—generic ownership with both specific events unregistered, or, only after #822 proves a stable opaque specific-event per-call ID and same-tool/replay stability, specific ownership with afterMCPExecution only and beforeMCPExecution unregistered—plus preserve-or-disable behavior for every unobserved path; then approves `product.md`/`tech.md` and explicitly moves GH-823 to `ready_to_implement`; Verify: GitHub approval/readiness evidence at the approved amendment head.
- [ ] `SP823-T3` Owner: implementation agent; Dependencies: `SP823-T2`; Done when: canonical Cursor host/identity parsing, null-path preservation, PII sanitization, fail-closed errors, session-init rejection, and a version/capability matrix implement B-001..B-006 and B-009..B-014; every supported Cursor entrypoint uses the frozen bounded byte reader before String allocation/serde, with exact-limit and one-byte-over behavior, while Cursor 3.12.17 sessionStart injection remains disabled and only an explicitly approved postToolUse context path may emit model-visible context; Claude/Codex remain unchanged; Verify: PR #914 identity/null/capability fixtures, per-entrypoint stdin exact/one-byte-over zero-side-effect fixtures, exact/one-byte-over tests for any approved limit, PII sentinel tests, `cargo fmt --check`, and `cargo check`.
- [ ] `SP823-T4` Owner: implementation agent; Dependencies: `SP823-T2`; Done when: postToolUse/postToolUseFailure and the B-016-selected MCP event variant map only verified object/string field types, the shared whole-stdin reader and raw generic/decoded field bounds run before classification, unknown names use verbatim generic capture, approved `tool_use_id` and MCP ownership produce exactly one canonical capture, and failed Read is preserved with an explicit outcome; generic ownership leaves both MCP-specific events unregistered, while MCP-specific ownership is unavailable without an approved stable opaque per-call ID, then registers only afterMCPExecution, leaves beforeMCPExecution unregistered, and makes generic MCP postToolUse zero-write; Task remains pre-tool/subagent-only until a real post-tool success is observed; unobserved Task-success/Write/Edit/Delete/failed-Shell paths remain generic or disabled exactly as approved; Verify: PR #914 generic/failure/MCP fixtures, whole-stdin plus raw-string/canonical/decoded exact-limit and one-byte-over boundaries, conditional accepted-event-set, two same-tool calls in one generation plus per-call replay/event-key/double-delivery tests, spill/replay/downstream, generic-capture, adapter, and zero-write boundary tests.
- [ ] `SP823-T5` Owner: implementation agent; Dependencies: `SP823-T2` and GH-825 merged with approved Cursor transcript fixtures; Done when: Cursor Stop first passes the shared whole-stdin limit before String/serde, then maps equal identity plus required non-empty string generation_id, approved status, and loop fields into the canonical Stop key; rejects over-limit input, missing/blank/wrong-typed generation_id, and unapproved error/nonzero/missing/null/wrong loop types before downstream work; and cannot invoke the Claude/Codex transcript reader/enqueue/spill/LLM before GH-825's reader is available; Verify: Stop stdin exact-limit/one-byte-over, PR #914 completed/aborted/loop-0 and token-presence fixtures, reader-spy prerequisite, new/replay/conflict key matrix, generation_id/status/loop invalid zero-call matrix, identity mismatch, and persistence tests.
- [ ] `SP823-T6` Owner: verification agent；Dependencies: `SP823-T3` `SP823-T4` `SP823-T5`；Done when: every B-001..B-016 mapping passes, PR #914 capability split and approved failure/MCP ownership are preserved, all unknowns stay visibly fail-closed, Claude/Codex fixtures remain unchanged, no user_email sentinel reaches any sink, and the owned `current-memory-contracts`/`cache-stable-injection` product+tech updates describe the shipped Cursor capability-specific emission/audit/layering behavior；Verify: focused tests, current-contract diff review, `cargo test`, `python3 checks/check_workflow.py --repo . --spec-dir specs/GH823`, and `git diff --check`.

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

- GH-822 PR #914 supplies the real MCP/generic/failure/capability evidence but
  still requires human adoption; unobserved variants and limits remain
  explicit fail-closed gates rather than guessed mappings.
- GH-825 is a hard prerequisite for Cursor summarize. Until it lands, Cursor
  transcript paths must not reach the existing Claude/Codex raw parser.
- PR #827 merged the original Draft packet; this amendment remains a Draft
  specification change and does not authorize runtime implementation,
  installation, final approval, or merge.
