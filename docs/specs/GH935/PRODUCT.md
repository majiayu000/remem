# Cross-Host Continuity Benchmark Product Spec

Status: Current contract (v1 infrastructure)
Issue: #935 (refs #849, #852, #385)

## Problem

#849 concluded remem's durable differentiation is cross-host unification, not
single-repo auto-memory. No benchmark answers whether history produced in
Claude Code can be reliably used by Codex on a new task, and vice versa, or
whether remem beats target-host native memory and exported handoff files.

## Goal

A bidirectional handoff suite:

```text
Claude Code -> remem -> Codex
Codex -> remem -> Claude Code
```

History and target phases run on different hosts with native session
continuity strictly isolated. Four primary conditions per direction:
`no_memory`, `target_host_native`, `exported_file` (maintenance cost counted),
and `remem_shared` (real capture pipeline in, normal SessionStart/MCP/Context
Bundle out). Diagnostic conditions cover native import, preloading, oracle
handoff, full transcript, and remem with/without host-native import.

`resolved_rate` on hidden tests is the primary outcome; secondary metrics,
stop-loss thresholds (`wrong_project_injection = 0`, `wrong_user_injection = 0`,
`source_private_session_leak = 0`, `stale_memory_followed <= 1%`,
`memory_hurt <= 2%`), and the claim gate (paired bootstrap, direction-specific
reporting, CI-includes-0 means directional only) are fixed in
`eval/cross-host/benchmark-charter.json`.

## v1 scope (this spec)

Infrastructure only, no runs:

- versioned charter, task schema, and run-artifact schema with full
  source-to-target attribution (session/event/memory/context refs, per-ref
  origin classification, scope and validity at target time);
- 12 task skeletons per direction covering the 12 required categories, all
  `skeleton_todo` with explicit TODOs instead of fabricated fixture data;
- artifact leak scanner blocking real HOME, host session store, auth, and
  benchmark-private path references;
- offline dry-run runner validating schemas, coverage, and condition matrix.

## Non-Goals

- No live bidirectional session bridge; no agent lease/queue management.
- No default injection of full source transcripts into the target host.
- Host-native memory is not treated as trusted canonical truth.
- #852 import mechanics are out of scope here.
- README must not cite any cross-host result until the claim gate passes.
