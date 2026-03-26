# SPEC: Core Refactor (No Backward Compatibility)

Date: 2026-03-26
Scope owner: remem core

## Goal

Perform a full core refactor in three areas without preserving backward compatibility:

1. Unify project identity and matching semantics.
2. Make pending observation processing recoverable (no hard delete on failure paths).
3. Unify MCP/API memory search/save behavior through a shared service layer.

## Why

Current behavior has architectural drift:

- Project matching logic is fragmented across multiple modules with inconsistent semantics.
- Pending observation failures can permanently lose raw data.
- MCP and REST API expose diverging save/search contracts and defaults.

## Non-Goals

- Preserving old REST payload field names/requiredness.
- Preserving legacy project-key compatibility (`last2`, `@hash` suffix matching, suffix wildcard rules).
- Historical data backfill to new project identity format.

## Design

### 1) Project Identity (single source of truth)

- Introduce `project_id` module as the only source for project key + SQL filtering helpers.
- New project key rule: canonical absolute cwd path string.
- All project filters use exact match semantics; remove suffix/legacy matching branches.

Affected paths:

- `src/project_id.rs` (new)
- `src/db.rs` (delegate identity APIs to `project_id`)
- `src/db_query.rs`
- `src/memory_search.rs`
- `src/search.rs`
- `src/temporal.rs`
- `src/entity.rs`

### 2) Pending Observation State Machine

- Extend `pending_observations` schema:
  - `status TEXT NOT NULL DEFAULT 'pending'`
  - `attempt_count INTEGER NOT NULL DEFAULT 0`
  - `next_retry_epoch INTEGER`
  - `last_error TEXT`
  - `updated_at_epoch INTEGER NOT NULL DEFAULT 0`
- Replace failure-path hard delete with explicit transitions:
  - recoverable error => release to pending with retry backoff
  - non-recoverable parse/quality error => mark failed (dead-letter style retention)
- Keep success path as delete-on-commit (processed records are removed).

Affected paths:

- `src/db.rs` (schema + migrations + indexes)
- `src/db_pending.rs` (claim/retry/fail APIs)
- `src/observe_flush.rs` (use retry/fail APIs)
- `src/doctor.rs` (pending health metrics include status)

### 3) Service Layer for Memory Save/Search

- Add `memory_service` module that centralizes:
  - input DTOs
  - search behavior (including multi-hop toggle)
  - save behavior (scope defaults + optional local backup)
- MCP and REST API become transport adapters calling shared service.
- REST request shape is aligned to service/MCP semantics; old API shape is dropped.

Affected paths:

- `src/memory_service.rs` (new)
- `src/mcp.rs`
- `src/api.rs`
- `src/lib.rs`

## Acceptance Criteria

1. Project filters are implemented by one helper path; no module-specific suffix matching remains.
2. No pending failure path in `observe_flush` hard-deletes records due to parse/quality/Task failures.
3. `save_memory` behavior is shared by MCP and REST through the same service function.
4. `search` behavior for MCP and REST is shared by the same service function.
5. `cargo test` passes.

## Risk

- Existing stored `project` values may become unreachable under new exact canonical path keys.
- Existing REST clients using old payload shape will fail after deployment.
- Pending table growth may increase due to retained failed rows.

## Verification

- `cargo test`
- Manual smoke:
  - `remem context --cwd <path>`
  - MCP `search` + `save_memory`
  - REST `/api/v1/search` + `/api/v1/memories`
