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

## Non-Goals

This path does not auto-write repository docs, generate skills, or create
runbooks. Exporting mature procedures to committed docs or skills should remain
an explicit review step.
