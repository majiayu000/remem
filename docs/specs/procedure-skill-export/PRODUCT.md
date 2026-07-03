# Procedure Skill Export Product Spec

Status: Current contract
Date: 2026-07-02

Tracking:
- Spec/tracking issue: #680
- Related: #671 (corrections → hook-enforced checks), #678 (memory pack export plumbing)

## Problem

Procedural memory promotes verified repeatable workflows into
`memory_type='procedure'` rows, but the value chain stops inside the store.
`docs/procedural-memory.md` declares exporting mature procedures to docs or
skills a non-goal that "should remain an explicit review step" — and that
review step has no surface. A multiply-verified procedure today is only a
retrieval hit; it cannot become a first-class agent capability (Claude Code
skill, Codex prompt, repo runbook) that fires deterministically instead of
depending on retrieval luck.

This is the #671 thesis applied to the procedure class: memory graduating
from passively injected text to actively executable capability.

## Goals

- Users can see which procedures are mature enough to externalize
  (`remem procedures list` with maturity signals).
- An explicit CLI command renders a mature procedure into a reviewable draft
  artifact: Claude Code skill, Codex prompt, or Markdown runbook. MCP may
  expose read-only procedure discovery later, but v1 export writes remain
  CLI-only. Committing the draft stays a human git action.
- Exported artifacts remain linked to their source procedure: when the
  procedure is invalidated or its verification goes stale, doctor flags the
  export as drifted.
- The export path is review-gated by construction: no background component
  can ever write these files.

## Non-Goals

- No automatic writes to `.claude/`, `AGENTS.md`, `CLAUDE.md`, or any
  high-context path from hooks, workers, or dream — a hard constraint
  enforced in code, not a default-off setting (SEC-13-class surface:
  auto-writing agent instruction files from memory content is a prompt-
  injection escalation path).
- No skill marketplace publishing; local repo files only.
- No new procedure promotion gates; the existing gates in
  `docs/procedural-memory.md` stay as-is.
- No LLM rewriting of procedure content at export time in v1; templates
  render the stored fields verbatim.

## User-Visible Behavior

- `remem procedures list` shows promoted procedures with verified run count,
  last verification time, branch/project binding, files touched, and
  confidence.
- `remem procedures export <id> --format claude-skill|codex-prompt|runbook-md
  [--out <dir>]` writes a draft file containing the command, reuse condition,
  preconditions (project/branch), and a provenance footer (source procedure
  id, evidence event ids, generation date, remem version).
- Every rendered file carries a marker identifying it as a remem-derived draft
  pending human review. For Claude skill exports, valid `SKILL.md` YAML
  frontmatter remains the first bytes of the file and the draft marker follows
  the closing frontmatter delimiter.
- The exporter refuses to overwrite reviewed or user-edited draft files.
- `remem doctor` reports exported artifacts whose source procedure is no
  longer active, whose verification freshness lapsed, or whose active source
  procedure changed after export.

## Acceptance Criteria

- Fixture procedure exports to all three formats; rendered output pinned by
  snapshot tests; provenance header and evidence ids present.
- Claude skill snapshot proves YAML frontmatter is first-line content and the
  draft warning marker is emitted only after frontmatter.
- Existing user-edited draft files are not overwritten; the CLI exits with an
  explicit error and instructions for choosing a new output path.
- Negative test: worker/dream/hook code paths cannot reach the export writer
  (module visibility or runtime guard with an explicit error).
- Doctor flags an export whose source procedure was invalidated, freshness-
  lapsed, or materially updated after export.
- `docs/procedural-memory.md` non-goal paragraph is replaced with the
  review-gated export contract.

## Risks

- Drafts drift from reality after commit: mitigated by the doctor back-link,
  not by auto-updating committed files (updating stays a human action).
- Users commit drafts without review: mitigated by the draft marker header
  and by never emitting directly into auto-loaded paths (default `--out` is
  a neutral directory, not `.claude/skills/`).
