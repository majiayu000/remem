# Preference Rule Compilation Product Spec

Status: Current contract
Date: 2026-07-11

Tracking:
- Spec/tracking issue: #671
- Related umbrella: #383 (repeated corrections promote to durable preferences)

## Problem

Historically, injected preference text was advisory: an agent could read "use
bun, not npm" in its context block and still run `npm install`. Published
measurement (TRACE, arXiv 2606.13174) shows recall-only preference systems
leave a large share of applicable preference checks violated, while compiling
corrections into mandatory runtime checks reduces violations to near zero
out-of-distribution.

remem uniquely owns both sides needed to close this gap: the preference store
(with reinforcement metadata) and the hook surface on every session. Current
enforcement is conditional: when rule compilation is enabled and an eligible
worker-built artifact exists, Claude Code PreToolUse(Bash) deterministically
evaluates supported command input and can warn or explicitly block. Phase 1
does not evaluate arbitrary prose or prompts, and Codex command enforcement is
unsupported because it has no pre-execution command hook.

## Goals

Phase 1 implementation status: the disabled-by-default configuration,
canonical SQLite reinforcement state, artifact and evaluator foundation,
worker-side compiler, user-visible hook enforcement, doctor reporting,
repeated-correction fixtures, and fixed-budget latency evidence are
implemented. The compiler uses persisted low-risk, source-trust, and review
eligibility, combines lifecycle-triggered non-lossy jobs with periodic
convergence sweeps, preserves same-predicate overrides, and resolves project
rules ahead of global rules. GH-813 identified that the global-owner filter
still accepts any non-null owner scope; the exact
`user`/`user:default`/no-target correction, exhaustive eligibility matrix, and
final closure remain pending. CLI management and warn-mode round trips are
implemented by #837; Claude Code PreToolUse and supported-host block persistence
are implemented by #839 at exact head
`905a55f7219459dd7b33a1805f0d4da27a97622f` (merged as
`f612b4a1ec4558ed6d2df85699cefb42109bdf7c`); doctor enforcement health is
implemented and reconciled by #840.

- Compile a small, high-confidence subset of preferences into deterministic
  rules that hooks can evaluate without an LLM.
- Warn (default) or block (opt-in, per rule) when a supported command
  invocation contradicts a compiled rule.
- Keep every compiled rule traceable to its source memory and reversible by
  the user.
- Keep compilation off the hook hot path: rules are produced by the background
  worker, hooks only evaluate.

## Non-Goals

- Compiling free-form, ambiguous, or low-confidence preferences. Those remain
  injection-only.
- Any LLM call inside a hook.
- Replacing preference injection; enforcement is additive.
- Cross-project or cross-host rule sharing in the first implementation.
- A general rules engine. The predicate language starts minimal.

## Behavior Invariants

1. Compilation eligibility is a closed, conjunctive contract. A source is
   eligible only when all of the following are true: memory type is
   `preference`; status is `active` and not expired; scope is `project` with
   `owner_scope='repo'` and the single resolved target
   `COALESCE(NULLIF(target_project, ''), NULLIF(owner_key, ''), project)` equal
   to the current project, or
   scope is `global` with `owner_scope='user'`, `owner_key='user:default'`, and
   no project target; source trust is one of
   `local_tool_output`, `repo_file`, or `user_prompt`; reinforcement state is
   machine-checkable, at or above the configured threshold, and independently
   `low` risk; the originating candidate is independently `low` risk with
   review status `approved`, `edited`, or `auto_promoted`; and policy evaluation
   succeeds with no matching `active` suppression targeting the memory,
   topic key, entity, or pattern. Unknown owner/scope/policy values, malformed
   suppression state, or any other missing or newly introduced value is
   ineligible until this contract and its tests explicitly classify it.
2. A compiled rule always records: source memory id, reinforcement count at
   compile time, compile timestamp, and the predicate.
3. Rule evaluation is deterministic and local: same event input, same verdict,
   no network, no DB write on the hot path.
4. Default action on match is a visible warning appended to hook output; block
   is opt-in per rule and never the compiled default.
5. When the source preference is superseded, suppressed, expired, or deleted, the
   compiled rule is removed on the next compile pass; a stale rule must never
   outlive its source memory by more than one compile cycle.
6. The user can list, disable, and re-enable compiled rules from the CLI, and
   disabling takes effect without restarting anything.
7. If the compiled rules file is missing or unreadable, hooks proceed without
   enforcement and log the condition at error level once per session; hooks
   never crash or block the agent because rule evaluation failed.

## Acceptance Criteria

- [x] Fixture suite of repeated-correction scenarios (package manager choice,
      forbidden commit trailers, forbidden commands) shows the warning fires on
      violation with compiled rules present and does not fire without them.
- [x] The existing hook latency benchmark passes both fixed budgets: enabled
      p95 is at most `15.0 ms`, and enabled-minus-disabled p95 delta is at most
      `1.0 ms`. MAD remains informational and cannot decide pass/fail.
- [x] `remem` CLI lists compiled rules with provenance; disable/enable,
      `set-action <rule_id> warn` (host optional), and
      `set-action <rule_id> block --host claude-code` round trips work and are
      covered across #837's management/warn tests and #839's supported Claude
      block test; the shared unsupported-pre-execution guard rejects block
      before persisting an override or compile job.
- [ ] Compiler eligibility has one complete positive fixture, independent
      negative coverage for every eligibility dimension, and critical
      cross-state coverage. Candidate risk and reinforcement risk are
      independently mutable; coverage is behavioral and does not snapshot SQL
      text.
- [x] Superseding, suppressing, expiring, or deleting the source preference
      removes the rule on the next compile pass, covered by tests.
- [x] Doctor reports compiled-rule count, last compile time, host capability,
      and the latest compile or evaluation error, covered by #840 human/JSON
      and privacy tests.

## Edge Cases

- Two compiled rules with contradictory predicates: project scope wins over
  global scope; within one scope the newest source memory wins. The dropped
  conflict is logged for review.
- A rule matching inside quoted or documentation text (for example a prompt
  that merely mentions `npm install`): first implementation only evaluates
  tool-invocation inputs (commands), not prose, to keep false positives low.
- Global-scope preferences: eligible only when the owner scope is explicit;
  project rules take precedence over global rules on conflict.

## Rollout Notes

Ship disabled by default behind a config flag; enable warn-mode by default only
after fixture evidence and one release of field soak. Block-mode remains
per-rule opt-in indefinitely.
