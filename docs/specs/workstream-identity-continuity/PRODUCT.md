# Workstream Identity Continuity Product Spec

Status: Current contract
Date: 2026-06-22

Tracking:
- Capability issue: #603

## Problem

remem workstreams are intended to keep long-running tasks resumable across
Claude Code and Codex sessions. Today, a task rename can split one real task
into multiple tracked workstreams because summary persistence treats the
LLM-provided workstream title as the primary identity signal.

The observed incident came from one Spellbook task in session
`019ed986-1081-7b21-92d5-99ab50923b1a`. The task started as
`agent-workflow`, was renamed to `flowguard`, and then was discussed as
`flowguard / run-guard`. remem created three workstreams:

| ID | Created local time | Title |
|---|---|---|
| `1140` | `2026-06-18 16:39:21 CST` | `agent-workflow Skill 生命周期工作流` |
| `1163` | `2026-06-18 19:29:59 CST` | `flowguard Skill 生命周期工作流` |
| `1167` | `2026-06-18 20:45:05 CST` | `flowguard / run-guard Skill 生命周期工作流` |

All three rows were produced by `summary-job` for the same session and same
Spellbook PR work. They later appeared as stale tracked work even after
Spellbook PR `#93` had merged. This forced manual cleanup and made the
SessionStart context less trustworthy.

## Goal

Preserve one canonical workstream identity across title drift, renames, and
reasonable wording changes while keeping prior titles auditable.

The product behavior should be:

1. A task can change display title without creating a second active task.
2. Prior titles stay searchable and explainable as aliases/history.
3. SessionStart renders one canonical active workstream per real task.
4. Manual governance can merge existing duplicates without losing session
   provenance.
5. Matching remains conservative enough to avoid silently merging unrelated
   tasks.

## User Stories

### Resume A Renamed Task

As a user, when I rename a skill or feature mid-thread, remem should still show
one workstream next time I start a session in that repo.

Example rename chain:

```text
agent-workflow Skill 生命周期工作流
flowguard Skill 生命周期工作流
flowguard / run-guard Skill 生命周期工作流
```

Expected result: one active workstream with the latest display title and the
older titles preserved as aliases.

### Inspect Why A Workstream Matched

As a user or maintainer, when remem updates an existing workstream instead of
creating a new one, logs or diagnostics should say why: same session link,
explicit alias, exact title, or conservative fuzzy fallback.

### Repair Existing Duplicates

As a maintainer, when older data already contains duplicates, I need a safe
governance path to merge duplicate rows into a canonical workstream while
preserving linked sessions and title history.

## Product Contract

### Canonical Workstream

Each real long-running task has one canonical workstream row. The row owns:

- stable identity;
- current display title;
- current progress, next action, and blockers;
- status lifecycle: `active`, `paused`, `completed`, or `abandoned`;
- links to all summary sessions that contributed to the workstream;
- title aliases/history.

### Display Title

The display title is mutable. It may update when a new summary gives a clearer
or more current title, but the old title must not be discarded.

### Alias History

Every accepted title for a canonical workstream becomes an alias/history item.
Aliases are used for retrieval, matching, diagnostics, and governance. Alias
history must preserve enough provenance to answer when a title first appeared
and which summary/session introduced it.

### Matching Order

Matching should prefer high-confidence identity signals before text matching,
but same-session matching is not unconditional. A long assistant session can
contain multiple unrelated user tasks, so a session link is safe only when it
selects one candidate and there is supporting continuity evidence.

1. Unique existing workstream linked to the same `memory_session_id`, with
   supporting continuity evidence.
2. Explicit workstream identity/ref, if present in a future summary contract.
3. Exact alias or exact normalized title match inside the same project/owner.
4. Conservative title similarity fallback.
5. Insert a new workstream only when no safe match exists.

### SessionStart Rendering

SessionStart context should render canonical active workstreams only. Alias
rows or merged duplicate rows must not appear as separate active tasks.

### Governance

Manual cleanup should support merging duplicate workstreams into a canonical
row. A merge must:

- move `workstream_sessions` links to the canonical row;
- preserve all aliases/title history;
- keep an auditable record of the merged duplicate IDs;
- not hard-delete source rows unless a separate destructive governance command
  explicitly does so.

## Non-Goals

- Do not remove automatic workstream creation from Stop summaries.
- Do not require agents to manually call `save_memory` or `update_workstream`
  for normal continuity.
- Do not auto-close or complete workstreams from GitHub PR state. External
  truth reconciliation is a separate feature.
- Do not silently merge unrelated tasks based only on broad words like `skill`,
  `workflow`, `fix`, or `review`.
- Do not make aliases globally unique across all projects.

## Success Metrics

| Metric | Current | Target |
|---|---|---|
| Spellbook rename-chain rows | 3 active/stale rows for one task | 1 canonical row |
| SessionStart duplicate workstream display | Possible | Prevented for canonicalized rows |
| Match diagnostics | Generic `upserted workstream id=...` | Logs include match reason |
| Alias auditability | Lost in title overwrite or split rows | Prior titles inspectable |

## Acceptance Criteria

- A regression test reproduces the Spellbook rename chain and results in one
  canonical active workstream.
- `workstream_sessions` links each contributing summary/session to the
  canonical row.
- Prior titles remain inspectable or searchable as aliases/history.
- SessionStart renders only one active workstream for the canonical task.
- Logs or diagnostics explain match reasons.
- Manual `update_workstream` continues to work.
- The implementation includes migration/schema tests for any new table or
  column.
- Focused workstream and summary tests pass, followed by `cargo fmt --check`
  and `cargo check`.

## Open Questions

1. Should the canonical display title always be the newest title, or should
   remem preserve the first title unless a high-confidence rename is detected?
   Recommendation: use newest title for active rows, preserve all prior titles
   as aliases.
2. Should manual merge mark duplicate rows as `completed`, `abandoned`, or a
   new status?
   Recommendation: avoid a new status initially; preserve provenance through a
   merge/audit table and remove merged rows from active context rendering.
3. Should the summary prompt emit a stable workstream ref?
   Recommendation: optional follow-up. First use deterministic DB-side
   continuity signals that do not depend on the LLM repeating an ID exactly.
