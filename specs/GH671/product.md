# Product Spec

## Linked Issue

GH-671

## Accepted Contract

The authoritative product contract is
`docs/specs/preference-rule-compilation/PRODUCT.md`.

This SpecRail packet hands the accepted #671 contract to implementation
planning. It does not replace the `docs/specs/` contract and does not approve
runtime implementation by itself.

## User Problem

Injected preference text is advisory. remem can remember a repeated correction
such as "use bun, not npm", inject it into future sessions, and still watch an
agent violate it because nothing checks tool input deterministically.

remem owns both the memory store and the hook surfaces, so high-confidence,
machine-checkable corrections can become local runtime checks instead of
remaining recall-only prose.

Phase 1 implementation status: this issue currently has the state foundation
only. It adds disabled-by-default configuration and canonical SQLite state for
preference reinforcement, user overrides, and diagnostics. User-visible rule
compilation, hook warnings/blocks, CLI rule management, and doctor reporting
remain pending and #671 must stay open.

## Goals

- Compile a small, high-confidence subset of repeated preferences into
  deterministic rules.
- Warn by default when a supported hook event contradicts a compiled rule.
- Allow block mode only through explicit per-rule user opt-in on hosts that can
  enforce before execution.
- Preserve provenance for every compiled rule: source memory, reinforcement
  count, compile time, predicate, and action.
- Let users list, disable, re-enable, and change action for compiled rules
  without editing generated files.
- Keep rule compilation off the hook hot path.

## Non-Goals

- Do not compile ambiguous, free-form, low-confidence, or prose-only
  preferences.
- Do not call an LLM, network service, or database write from hook-side rule
  evaluation.
- Do not replace preference injection; compiled rules are additive.
- Do not share compiled rules across projects in v1.
- Do not claim block-mode enforcement on hosts without pre-execution hook
  support.

## Behavior Invariants

1. P1: A preference is eligible only when it is active, reinforced at or above
   the configured threshold, low-risk, project-scoped or explicitly
   global-scoped, and machine-checkable.
2. P2: Every compiled rule records source memory id, reinforcement count at
   compile time, compile timestamp, predicate kind, predicate data, action, and
   user override state.
3. P3: Hook-side evaluation is deterministic and local: the same event input
   and rule artifact produce the same verdict with no LLM call, network call,
   or database write.
4. P4: The default action for a compiled rule is a visible warning. Block mode
   is never inferred and must be explicitly selected by the user for a rule and
   supported by the current host.
5. P5: A compiled rule whose source memory is superseded, suppressed, expired,
   or deleted is removed on the next compile pass.
6. P6: User disable, enable, and action overrides persist across artifact
   deletion and regeneration.
7. P7: If the rules artifact is missing, unreadable, corrupt, or contains an
   unsupported predicate, hooks proceed without blocking and record an
   error-level diagnostic that `remem doctor` can surface.
8. P8: Hosts without pre-execution command hooks report unsupported block-mode
   enforcement instead of implying protection exists.

## Acceptance Criteria

- [ ] Repeated-correction fixtures cover package-manager choice, forbidden
      commit trailers, and forbidden commands; violations warn with compiled
      rules and do not warn without them.
- [ ] p95 hook latency with rule evaluation enabled is unchanged within
      measurement noise on the existing latency benchmark.
- [ ] `remem rules list` shows provenance, effective action, disabled state,
      and source memory for each compiled rule.
- [ ] Disable, enable, and `set-action warn|block` round trips are covered by
      tests and take effect after the next artifact build without restart.
- [ ] Superseding, suppressing, expiring, or deleting a source preference
      removes the derived rule on the next compile pass.
- [ ] Doctor reports compiled-rule count, last compile time, host enforcement
      capability, and the most recent compile or evaluation error.

## Edge Cases

- Contradictory rules: keep the newest authoritative source memory and log the
  dropped conflict for review.
- Quoted examples and documentation text: v1 evaluates supported tool command
  input, not arbitrary prose, to avoid false positives.
- Global preferences: eligible only when global scope is explicit; project
  rules take precedence on conflict.
- Unsupported host: warnings or blocks that require pre-execution command
  hooks remain unavailable and are reported honestly.

## Rollout Notes

Spec approval is still a human gate. Implementation should ship behind a
disabled-by-default config flag, then enable warn mode only after fixture and
latency evidence. Block mode remains opt-in per rule indefinitely.
