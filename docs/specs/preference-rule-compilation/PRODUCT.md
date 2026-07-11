# Preference Rule Compilation Product Spec

Status: Current contract
Date: 2026-07-11

Tracking:
- Spec/tracking issue: #671
- Related umbrella: #383 (repeated corrections promote to durable preferences)

## Problem

Injected preference text is advisory. An agent can read "use bun, not npm" in
its context block and still run `npm install`. Published measurement (TRACE,
arXiv 2606.13174) shows recall-only preference systems leave a large share of
applicable preference checks violated, while compiling corrections into
mandatory runtime checks reduces violations to near zero out-of-distribution.

remem uniquely owns both sides needed to close this gap: the preference store
(with reinforcement metadata) and the hook surface on every session. Today the
hook surface only injects prose; it never enforces anything the memory system
has learned.

## Goals

Phase 1 implementation status: the disabled-by-default state, artifact and
evaluator foundation, and worker-side compiler are implemented. User-visible
hook enforcement, CLI management, doctor reporting, fixtures, and latency
evidence remain pending.

- Compile a small, high-confidence subset of preferences into deterministic
  rules that hooks can evaluate without an LLM.
- Warn (default) or block (opt-in, per rule) when a tool invocation or prompt
  contradicts a compiled rule.
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

1. Only preferences that are machine-checkable, reinforced at or above a
   configurable threshold, low-risk, and backed by an accepted persisted source
   trust class are eligible for compilation.
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

- [ ] Fixture suite of repeated-correction scenarios (package manager choice,
      forbidden commit trailers, forbidden commands) shows the warning fires on
      violation with compiled rules present and does not fire without them.
- [ ] p95 hook latency with rule evaluation enabled is unchanged within
      measurement noise on the existing latency benchmark.
- [ ] `remem` CLI lists compiled rules with provenance; disable/enable
      round-trip works and is covered by a test.
- [x] Superseding, suppressing, expiring, or deleting the source preference
      removes the rule on the next compile pass, covered by tests.
- [ ] Doctor reports compiled-rule count, last compile time, and last
      evaluation error if any.

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
