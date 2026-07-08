# Product Spec

## Linked Issue

GH-680

## Accepted Contract

The authoritative product contract is
`docs/specs/procedure-skill-export/PRODUCT.md`.

This SpecRail packet hands the existing #680 contract to workflow tracking. It
does not replace the `docs/specs/` contract and does not approve runtime
implementation by itself.

## User Problem

Procedural memory can promote verified repeatable workflows into
`memory_type='procedure'` rows, but there is no explicit surface to graduate a
mature procedure into a reviewable Claude skill draft, Codex prompt, or repo
runbook. The value stays dependent on retrieval instead of becoming a
deterministic agent capability.

## Goals

- List mature procedures with verification and freshness signals.
- Export one mature procedure through an explicit CLI command into a reviewable
  draft artifact.
- Keep draft files linked to their source procedure so doctor can flag drift.
- Prevent hooks, workers, dream jobs, or MCP write paths from exporting high
  context artifacts automatically.

## Non-Goals

- No automatic writes to `.claude/`, `.codex/`, `AGENTS.md`, `CLAUDE.md`, or
  skill roots.
- No marketplace publishing.
- No new procedure promotion gates in this issue.
- No LLM rewriting of procedure content at export time.

## User-Visible Behavior

- `remem procedures list` shows promoted procedures and maturity signals.
- `remem procedures export <id> --format claude-skill|codex-prompt|runbook-md
  [--out <dir>]` writes a draft with command, reuse condition, preconditions,
  and provenance.
- Claude skill exports keep valid frontmatter as the first bytes and place the
  draft marker after the closing frontmatter delimiter.
- Existing reviewed or user-edited draft files are not overwritten silently.
- `remem doctor` flags exported artifacts whose source procedure changed,
  became inactive, or lost freshness.

## Acceptance Criteria

- [x] Fixture procedure exports to all three formats with snapshot coverage,
      provenance header, and evidence ids.
- [x] Claude skill snapshot proves frontmatter is first-line content and the
      description includes a bounded reuse-condition summary.
- [x] Export rejects ineligible source memories and source text that fails
      secret or instruction-pattern scan.
- [x] Existing user-edited draft paths are refused with actionable errors.
- [x] Worker/dream/hook paths cannot reach the export writer.
- [x] Doctor reports drifted exports.
- [x] `docs/procedural-memory.md` describes the review-gated export contract.

## Edge Cases

- Default output path is neutral and not auto-loaded by agent tooling.
- User-edited draft files are treated as reviewed and never overwritten by
  default.
- Doctor drift is based on source procedure state, not on hashing user-edited
  exported files.

## Rollout Notes

This touches high-context agent instruction surfaces. Implementation must keep
file writes CLI-only, explicit, and review-gated by construction.
