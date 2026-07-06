# Project Memory Pack Product Spec

Status: Current contract
Date: 2026-07-02

Tracking:
- Spec/tracking issue: #678
- Related contracts: #672 (source trust classes), #381 (invalidate-never-delete), #673 (byte-stable rendering)

## Problem

remem memory is trapped in a single machine's encrypted SQLite store. There is
no way to share curated project memory with a teammate, commit a project's
durable memory to the repo so a new contributor's first session starts warm,
or migrate selectively between machines without a full-DB backup restore.

This is the one team-facing capability in the competitive gap matrix that no
local-first competitor ships (claude-mem and Engram are single-user stores;
mem0 and Letta solve it only via hosted cloud). A git-committable memory pack
keeps the local-first principle while opening the team/onboarding scenario.

## Goals

- `remem export --project` produces a deterministic, git-diffable pack of
  active repo-owned startup memories that a repo can commit and review like
  code. Tool/domain/user/workstream/session-owned rows are excluded even when
  their legacy `scope` value is `project`.
- `remem import --pack` merges a pack into the local store safely: dedup by
  identity, never resurrecting locally suppressed or invalidated memories,
  and never granting imported content local-grade trust.
- Round-trip integrity: export → import into an empty store → export again
  yields an identical pack.
- Injected context sourced from a pack stays auditable: `remem why` and
  doctor can attribute a memory to the pack it came from.

## Non-Goals

- No sync service, no CRDT, no automatic bidirectional sync — git is the
  transport.
- No user-scoped or cross-project memory in packs; project scope only in v1
  (user profile stays local).
- No encryption of the pack itself; it is meant to be committed. Secret
  redaction runs at capture time, and export re-runs the redaction scan as a
  final gate.
- No automatic import: packs are only ingested by an explicit CLI/MCP call.

## User-Visible Behavior

- `remem export --project <p> [--pack <dir>]` writes the pack (JSONL data +
  generated Markdown index for humans). Re-export with unchanged memory state
  is byte-identical, so `git diff` on the pack shows real memory changes only.
- `remem import --pack <dir> [--dry-run]` reports adds / dedups / skips
  (suppressed, invalidated, conflicting) before writing; without `--dry-run`
  it applies the merge and prints the same report.
- Imported memories participate in retrieval and injection like local ones,
  but carry a `pack` source trust class: they cannot silently auto-promote
  into higher-trust surfaces, and `remem why` shows the pack origin.
- Doctor shows imported-pack counts and origins.

Implementation note: deterministic pack export and the redaction fail-loud gate
are implemented. Pack import dry-run planning now validates pack integrity and
reports add/dedup/skip/conflict/quarantine categories without mutating the
store. Active import, round-trip identity, `pack` trust-class wiring, and
doctor/why attribution remain pending follow-up implementation.

## Team Onboarding Walkthrough (target README content)

1. Maintainer runs `remem export --project . --pack .remem-pack/` and commits
   the pack.
2. New contributor installs remem, runs `remem import --pack .remem-pack/`.
3. Their first session starts with the project's decisions, bugfix rationale,
   and architecture facts injected — no re-explaining, no hand-maintained
   CLAUDE.md dump.

## Acceptance Criteria

- Deterministic pack: exporting twice with unchanged memory state is
  byte-identical (test).
- Round-trip fixture: export → fresh-store import → re-export identity
  (test).
- Merge safety: importing a pack containing a memory that is locally
  suppressed or invalidated does not resurrect it (test).
- Imported memories carry a `pack` source trust class consumed by the
  auto-promote/injection gates (wired to the #672 vocabulary).
- Export re-runs the redaction scan; a seeded secret blocks the export with
  an error naming the offending row — no silent skip (U-29).
- README team-onboarding walkthrough exists and matches actual CLI behavior.

## Risks

- Pack contents are third-party input on import: instruction-injection text
  committed by a teammate flows into future sessions. Mitigated by the #672
  write-time scan applying to imports and by the `pack` trust class capping
  automatic promotion; residual risk is accepted for repo-trusted
  collaborators and documented.
- Format lock-in: the pack format becomes a public contract once committed to
  repos. Mitigated by a versioned manifest and an explicit compatibility
  policy in TECH.md.
