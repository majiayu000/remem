# Session Observatory Product Spec

Status: Current contract
Date: 2026-08-20

## Problem

Remem already preserves raw conversation occurrences, captured tool evidence,
session summaries, observations, and durable memories. Its local app is still
memory-centric: a user can search and govern memories, but cannot answer the
basic session-review questions in one place:

- What did I ask?
- What did the agent say it understood?
- What actions did it take?
- What was the outcome?

The existing `session_summaries` row is session-level and intentionally geared
toward memory extraction. It cannot represent multiple conversational turns or
support turn-level activity statistics without reparsing raw evidence on every
request.

## Goals

1. Add a rebuildable structured turn layer above the immutable raw archive.
2. Preserve evidence references for every projected field.
3. Make missing or partial capture visible instead of presenting inferred
   activity as complete.
4. Expose read-only session activity and activity statistics through the local
   REST API and Remem app.
5. Add an Overview and Sessions workspace to the existing Remem app while
   preserving memory search, save, governance, timeline, workstream, and
   activation features.

## Non-Goals

- Do not replace, rewrite, or delete `raw_messages`, `captured_events`,
  `session_summaries`, observations, or memories.
- Do not install a second hook, transcript adapter, database, or `.noter`
  journal.
- Do not claim that every host captures every low-value read/search tool event.
- Do not add an LLM call to the synchronous hook or API request path.
- Do not turn activity statistics into cognitive or growth scoring; that stays
  downstream in Refine.

## Product Model

Remem exposes three distinct layers:

| Layer | User question | Source of truth |
| --- | --- | --- |
| Raw evidence | What exactly occurred? | `raw_messages`, `captured_events` |
| Session activity | What happened in each conversational turn? | Rebuildable `session_turns` and `session_turn_actions` |
| Durable memory | What should be recalled later? | summaries, observations, candidates, memories |

### Turn Card

Each turn presents:

- the user's original message;
- an optional understanding statement from assistant text before actions;
- an ordered, compact action list backed by captured event IDs;
- an optional action narrative;
- a result summary and conservative status;
- capture health and source references.

Missing understanding or result text is rendered as unavailable. A turn with
incomplete action capture is marked `partial` or `unavailable`, never `full`.
All transcript-derived text is passed through the shared sensitive-text
redactor before it becomes API-visible; truncation alone is not a privacy
boundary.

### Activity Statistics

The Overview may show:

- sessions and turns in the selected window;
- result status distribution;
- capture-health distribution;
- project and host activity;
- captured action count and tool distribution;
- recent sessions.

Memory counts remain a separate operational section and retain their existing
meaning.

## Compatibility

- Existing API routes and fields remain compatible.
- New database tables are additive and rebuildable.
- Existing raw sessions are not synchronously backfilled during migration.
- Projecting a session is idempotent for an unchanged source digest.
- Projection and API responses have explicit message, action, turn, cursor,
  and statistics-window bounds. Truncation is machine-visible.
- Session-list pages may be sparse while their bounded raw scan advances; a
  non-null cursor is authoritative even when the current page is empty.
- Session message counts are capped and explicitly marked when truncated;
  `first_epoch` is unavailable rather than misleading on a truncated count.

## Acceptance Criteria

- A migration creates normalized turn and action tables with evidence links,
  projection version, source digest, and capture health.
- A deterministic projector converts an exact raw-session tuple into ordered
  turns without modifying raw evidence.
- Reprojecting unchanged input is a no-op; changed input atomically replaces
  only the affected session projection.
- Missing assistant text and missing action evidence remain explicit null or
  non-full health states.
- Authenticated REST endpoints list raw-backed sessions, return turn activity,
  and return bounded activity statistics.
- The local Remem app provides Overview, Sessions, and turn-detail views and
  preserves the existing memory/admin capabilities.
- Focused Rust and Node tests plus desktop/mobile browser inspection pass.
