# Product Spec

## Linked Issue

GH-672

## Accepted Contract

The authoritative product contract is
`docs/specs/memory-poisoning-defense/PRODUCT.md`.

This SpecRail packet hands the existing #672 contract to workflow tracking. It
does not replace the `docs/specs/` contract and does not approve runtime
implementation by itself.

## User Problem

remem injects persisted memory into shell-capable coding agents. Captured
events can include attacker-influenced content from tool output, fetched web
pages, files, and command output. If instruction-like content is promoted into
memory, it becomes persistent prompt injection that can reappear at
SessionStart or UserPromptSubmit until someone manually suppresses it.

## Goals

- Quarantine instruction-like memory candidates at write time instead of
  promoting them.
- Persist a deterministic source trust class that promotion gates can use.
- Re-scan at injection time as defense in depth and report drops loudly.
- Keep explicit review as the false-positive escape hatch.

## Non-Goals

- No LLM or network call in the injection path.
- No broad semantic intent classifier in the first slice.
- No general secrets scanner replacement; existing redaction remains.
- No silent bulk rewrite of all historical memories in the first slice.

## Behavior Invariants

1. New candidates derive source trust from provenance rather than model output.
2. Instruction-pattern matches become quarantined review items, never
   auto-promoted active memories.
3. Auto-promotion consumes the trust class and never promotes external content
   by confidence alone.
4. Render-time defense drops matching unacknowledged content and records enough
   detail to diagnose the drop.
5. A reviewer can approve a false positive only with an explicit pattern
   acknowledgement.

## Acceptance Criteria

- [ ] Poisoned-event fixtures prove instruction payload memories do not reach
      rendered context and each block is logged with pattern and provenance.
- [ ] Candidate or memory schema carries source trust class and auto-promote
      tests cover every trust boundary.
- [ ] Doctor reports quarantine count, pattern-set version, and last injection
      drop details.
- [ ] Quarantined candidates can be reviewed and approved only with explicit
      acknowledgement of the matched pattern.

## Edge Cases

- Legitimate security lessons that quote prompt-injection strings are
  quarantined first and require explicit acknowledgement.
- Mixed provenance uses the lowest trust class among supporting evidence.
- Existing rows without trust metadata keep a conservative compatibility
  default until a later backfill is designed.

## Rollout Notes

This is security-sensitive and multi-module. Implementation must preserve
human review gates, avoid silent degradation, and keep the pattern table
deterministic and versioned.
