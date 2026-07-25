# Task Plan

Status: Draft, needs human approval before implementation.

## Linked Issue

GH-852（Refs #852；Epic #849）

## Spec Packet

- Product: `product.md`
- Tech: `tech.md`

## Implementation Tasks

- [ ] `SP852-T1` Owner: maintainer or human-supervised agent with a real Claude Code installation; Dependencies: none (evidence-only, no repo runtime changes); Done when: PoC-1 per tech spec §1/§2 runs in an isolated HOME + `REMEM_DATA_DIR`, records Claude binary version, effective-settings source resolution for `autoMemoryDirectory`（user/policy/`--settings` accepted, project/local rejected）, actual startup-load window and capacity behavior of `MEMORY.md`/topic files, hook failure propagation, and interaction with the existing hardcoded `sync_to_claude_memory` path (`src/context/claude_memory/paths.rs`, `runtime.rs`) including the `REMEM_DISABLE_NATIVE_MEMORY_SYNC` / `REMEM_NATIVE_MEMORY_MAX_BYTES` guards; evidence separates observed facts from design inference and names unsupported versions; no real user memory is read, copied, or committed; Verify: maintainer review of the immutable evidence bundle linked from the GH-852 spec review thread.
- [ ] `SP852-T2` Owner: maintainer or human-supervised agent with a real Codex CLI installation; Dependencies: none (evidence-only); Done when: PoC-2 per tech spec §1/§3 documents the real `~/.codex/memories/` layout from an isolated HOME with format fingerprints, empty/unknown-version/concurrent-write behavior, and source-tree hashes proving read-only access; observed formats become the closed set the detector may accept; Verify: maintainer review of the evidence bundle plus before/after source-tree hash equality.
- [ ] `SP852-T3` Owner: maintainer or human-supervised agent; Dependencies: none (evidence-only); Done when: the Codex `hooks.json` coverage audit per tech spec §6 records the event/schema matrix from isolated real sessions for core `remem install --target codex` and plugin `activate-codex.js` activation entrypoints plus a plugin-only MCP control, compares against Claude PreToolUse/PostToolUse observe semantics (`src/install/config.rs` `HookStrategy::ClaudeCode` branch), and states a go/no-go conclusion with missing events/fields listed; no `build_hooks` or installed-event-set change is made; Verify: maintainer review of the audit document; `git diff` shows no runtime changes from this task.
- [ ] `SP852-T4` Owner: human maintainer; Dependencies: `SP852-T1` `SP852-T2` `SP852-T3`; Done when: a human reviews the three evidence bundles at the exact spec head, makes the security/threat-model decision for host data ownership, confirms the Claude takeover design remains explicit opt-in with a verified reverse commit point (atomic delivery-block removal restoring `hook_only`, uninstall honoring receipts, never overwriting user-modified values), confirms the native topic-file input path stays no-go until it meets the redaction/candidate/error contract, then approves `product.md`/`tech.md` and explicitly moves GH-852 to `ready_to_implement`; Verify: GitHub approval and readiness evidence on issue #852 at the approved head.
- [ ] `SP852-T5` Owner: implementation agent; Dependencies: `SP852-T4` and `SP852-T2` evidence adopted; Done when: `remem import codex-memories` implements tech spec §3–§5 — new `ImportAction::CodexMemories` (extending `src/cli/archive_types.rs`, not reusing backup/markdown best-effort or pack direct-active semantics), two-phase discovery/frozen-plan/apply with `--expect-plan-digest`, pre-persistence secret boundary, `source=codex_native` provenance and idempotent identity, records landing only in `pending_review`/`quarantined`/`dedup`/`blocked` (never active memories), fail-visible diagnostics for absent/unreadable/unsupported sources, and doctor status without body output; Verify: `cargo fmt --check`, `cargo check`, `cargo test import`, focused dry-run/apply/rename-dedup/quarantine/rollback/source-hash-unchanged tests per tech spec Product-to-Test Mapping B-005..B-011/B-014/B-017/B-018.
- [ ] `SP852-T6` Owner: implementation agent; Dependencies: `SP852-T4` and `SP852-T1` evidence adopted with an explicit per-host go decision; Done when: the Claude `autoMemoryDirectory` opt-in takeover implements tech spec §2 — settings resolver with scope rejection, dry-run conflict reporting without overwrite, receipt/prepared-manifest/marker-bounded `MEMORY.md` delivery block with atomic single commit point, SessionStart manifest exclusion, `hook_only`/`native_active`/`inconsistent` recovery states, rollback via atomic block removal, and closure audit of `src/observe/native.rs` so remem-owned files are never self-ingested and no direct-active/warning-only path widens; a human explicitly confirms before the first write to any real user `~/.claude` settings surface, and a PoC-proven no-go for a host/version keeps `hook_only` with the reason documented instead of shipping the bridge; Verify: `cargo fmt --check`, `cargo check`, focused install/ownership/recovery/rollback/no-double-injection tests per Product-to-Test Mapping B-002..B-004/B-016/B-019, plus recorded human confirmation.
- [ ] `SP852-T7` Owner: verification agent; Dependencies: `SP852-T5` `SP852-T6`; Done when: every product invariant in the tech spec Product-to-Test Mapping passes, active memories are proven unchanged by import fixtures, no real native-memory content or secrets appear in fixtures/logs, docs/version-sync surfaces touched by implementation are updated, and the Planned Changes Manifest still matches the shipped file set; Verify: `cargo test`, `cargo clippy --all-targets -- -D warnings`, `python3 checks/check_workflow.py --repo . --spec-dir specs/GH852`, `python3 scripts/ci/check_plugin_version_sync.py`（若触及版本 surfaces）, `git diff --check`.

No implementation task (`SP852-T5` or later) may start until `SP852-T4` has
fresh human approval at the exact spec head. Autonomous agents may not infer
approval from this packet, CI status, merged evidence PRs, or the existence of
the spec PR. Any action that writes to a real user `~/.claude` or `~/.codex`
surface is explicit opt-in, receipt-tracked, and reversible; conflicts with
user-owned values stop and report instead of overwriting.

## Parallelization

- `SP852-T1`/`SP852-T2`/`SP852-T3` are independent evidence tasks and may run in
  parallel; they own evidence bundles, not repository runtime files.
- `SP852-T4` is a serialized human gate.
- After `SP852-T4`, `SP852-T5` and `SP852-T6` may run in isolated worktrees with
  disjoint file ownership per the tech spec Planned Changes Manifest
  (import/CLI/doctor paths vs Claude install/context paths).
- `SP852-T7` is read-only verification after all implementation tasks finish.

## Verification

- `python3 checks/check_workflow.py --repo .`
- `python3 checks/check_workflow.py --repo . --spec-dir specs/GH852`
- `git diff --check`
- Implementation-only after the human gate: `cargo fmt --check`, `cargo check`,
  focused tests, `cargo test`.

## Handoff Notes

- PR #903 merged the recovered `product.md`/`tech.md`; PR #905 merged the
  research report the issue cites. This packet remains a Draft specification
  and does not authorize runtime implementation, installation, approval, or
  merge.
- The hooks audit (`SP852-T3`) is evidence-only; any runtime hook change it
  motivates requires a separate issue/spec with its own human gates.
- GH-855 poisoning-defense runtime is still spec-only on main; GH-852
  implementation must not assume its defenses exist (tech spec Risks).
