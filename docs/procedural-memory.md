# Procedural Memory

Procedural memory stores repeatable workflows, not one-off events. The write
path is intentionally gated because a bad procedure can teach the agent to keep
doing the wrong thing.

## Promotion Criteria

A trace-derived procedure may be promoted only when all gates pass:

- at least two successful verified runs
- every run has a raw source event id
- every run is within the verification freshness window
- project, branch, workflow key, and command all match
- source files touched by the workflow are preserved as metadata

One-off successes, mixed-project traces, mixed-branch traces, failed runs, stale
verification, or missing source refs do not promote.

## Stored Memory

Promoted procedures are written as `memory_type='procedure'` with project scope.
The promoted memory keeps:

- branch, so procedures do not leak across unrelated branches by default
- evidence event ids from the verified source traces
- files touched by the verified runs
- command and reuse condition in the memory content
- confidence derived from repeated verified event count

Procedural memory is still ordinary remem memory. It participates in retrieval
only after the gates pass; raw traces remain evidence, not prompt context.

Use `remem procedures list` to inspect promoted procedures and their maturity
signals before deciding whether to externalize one through a later review step.

## Review-Gated Export

Use `remem procedures export <id> --format claude-skill|codex-prompt|runbook-md
[--out <dir>]` to render a mature procedure as a draft artifact. The default
output directory is `remem-drafts/`, a neutral location that is not loaded as an
agent instruction path.

Export is intentionally CLI-only. Worker, dream, hook, and MCP paths cannot
write procedure drafts, and committing or moving a draft into an active docs or
skill location remains a human review step.

The exporter refuses high-context destinations such as `.claude/`, `.codex/`,
`AGENTS.md`, `CLAUDE.md`, repo-local `skills/`, repo-local `.agents/skills/`,
and plugin `skills/` roots. It also scans rendered fields for secrets and
instruction patterns before opening the output file.

Existing reviewed or user-edited drafts are not overwritten. Only an unchanged
remem-generated draft with a matching `procedure_exports` registry row and
content digest can be replaced, and only when the user passes
`--overwrite-generated`.

The `procedure_exports` registry records the source procedure snapshot and the
draft content digest at export time. `remem doctor` reports exports whose source
procedure is now inactive, whose verification freshness lapsed, or whose active
source procedure changed after export.
