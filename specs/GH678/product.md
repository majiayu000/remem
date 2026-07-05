# Product Spec

## Linked Issue

GH-678

## Accepted Contract

The authoritative product contract is
`docs/specs/project-memory-pack/PRODUCT.md`.

This SpecRail packet hands the existing #678 contract to workflow tracking. It
does not replace the `docs/specs/` contract and does not approve runtime
implementation by itself.

## User Problem

Curated project memory currently lives in one local encrypted SQLite store.
Teams cannot commit a reviewable project memory pack, selectively onboard a
teammate, or migrate only project-scoped durable memory without a full database
backup restore.

## Goals

- Export active repo-owned startup memories into a deterministic,
  git-diffable pack.
- Import a pack safely without resurrecting local suppressed or invalidated
  memories.
- Preserve auditability so imported memories can be traced to their pack
  origin.
- Prove export/import round trips are byte-stable.

## Non-Goals

- No sync service or automatic bidirectional synchronization.
- No user-scoped or cross-project memory packs in v1.
- No encryption of the pack itself; it is intended for review before commit.
- No automatic import path from hooks, workers, or background jobs.

## User-Visible Behavior

- `remem export --project <p> [--pack <dir>]` writes `pack.json`,
  `memories.jsonl`, and `INDEX.md`.
- Re-exporting unchanged memory state produces identical bytes.
- `remem import --pack <dir> [--dry-run]` reports adds, dedups, suppressions,
  invalidations, conflicts, and quarantines before writing.
- Imported rows carry pack provenance visible through `remem why` and doctor.

## Acceptance Criteria

- [ ] Exporting twice with unchanged memory state is byte-identical.
- [ ] Export -> fresh-store import -> export produces identical pack bytes.
- [ ] Import does not resurrect locally suppressed or invalidated memories.
- [ ] Imported memories carry a `pack` source trust class consumed by the
      #672 trust vocabulary and gates.
- [ ] Export re-runs redaction and fails loudly on seeded secret content.
- [ ] README documents the team-onboarding workflow.

## Edge Cases

- Unknown future pack format versions fail closed with actionable errors.
- State-key conflicts prefer local memory and route pack content to review
  rather than silently overwriting either side.
- Active import depends on #672 trust and quarantine support; export and
  dry-run planning may ship earlier.

## Rollout Notes

The pack format becomes a public contract. The first implementation must use a
versioned manifest, stable serialization, and explicit compatibility policy.
