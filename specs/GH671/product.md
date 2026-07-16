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

Phase 1 implementation status: `SP671-T1`, `SP671-T2`, and `SP671-T4`
through `SP671-T7` are implemented. The core T3 compiler is present, including
disabled-by-default configuration, canonical SQLite state, evidence-backed
reinforcement, the artifact/evaluator foundation, and deterministic
worker-side compilation driven by lifecycle jobs and periodic convergence
sweeps. #837 provides the CLI management evidence and #840 provides the doctor
evidence. GH-813 identified that global ownership is still filtered too
broadly; its exact owner correction and exhaustive eligibility matrix keep T3
and the final T8 closure incomplete, so #671 must stay open.

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

1. P1: Eligibility is conjunctive and closed. A source is eligible only when
   its memory type is `preference`; it is active and unexpired; scope is
   `project` with `owner_scope='repo'` and the resolved target
   `COALESCE(NULLIF(target_project, ''), NULLIF(owner_key, ''), project)` equal
   to the current project, or scope is
   `global` with `owner_scope='user'`, `owner_key='user:default'`, and no
   project target; source trust is `local_tool_output`, `repo_file`, or
   `user_prompt`; reinforcement is machine-checkable, at or above the threshold,
   and independently `low` risk; the originating candidate is independently
   `low` risk with review status `approved`, `edited`, or `auto_promoted`;
   policy evaluation succeeds; and no matching `active` memory/topic-key/
   entity/pattern suppression exists. Unknown owner/scope/policy values,
   malformed suppression state, and all other missing or newly introduced
   values are ineligible until the contract and tests explicitly classify them.
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

- [x] Repeated-correction fixtures cover package-manager choice, forbidden
      commit trailers, and forbidden commands; violations warn with compiled
      rules and do not warn without them.
- [ ] Compiler eligibility has one complete positive fixture, independent
      negative coverage for every eligibility dimension, and critical
      cross-state coverage; candidate risk and reinforcement risk are
      independently mutable and tests do not snapshot SQL text.
- [x] The existing hook latency benchmark passes both fixed budgets: enabled
      p95 is at most `15.0 ms`, and enabled-minus-disabled p95 delta is at most
      `1.0 ms`. MAD remains informational and cannot decide pass/fail.
- [x] `remem rules list` shows provenance, effective action, disabled state,
      and source memory for each compiled rule, covered by #837.
- [x] Disable, enable, and `set-action warn|block` round trips are covered by
      #837 tests and take effect after the next artifact build without restart.
- [x] Superseding, suppressing, expiring, or deleting a source preference
      removes the derived rule on the next compile pass.
- [x] Doctor reports compiled-rule count, last compile time, host enforcement
      capability, and the most recent compile or evaluation error, covered by
      #840 human/JSON, capability, corruption, recovery, and privacy tests.

## Edge Cases

- Contradictory rules: project scope wins over global scope; within the same
  scope, keep the newest authoritative source memory and log the dropped
  conflict for review.
- Quoted examples and documentation text: v1 evaluates supported tool command
  input, not arbitrary prose, to avoid false positives.
- Global preferences: eligible only when global scope is explicit; project
  rules take precedence on conflict.
- Unsupported host: warnings or blocks that require pre-execution command
  hooks remain unavailable and are reported honestly.

## Rollout Notes

Spec approval is still a human gate. Implementation ships behind a
disabled-by-default config flag. Fixture and latency evidence now pass the
fixed acceptance budgets; any future warn-mode default change remains a
separate human decision. Block mode remains opt-in per rule indefinitely.
