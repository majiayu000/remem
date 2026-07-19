# Raw Session Ingestion

Status: Current contract
Refs: GH-871, GH-720

## Goal

Preserve every supported Claude Code and Codex transcript occurrence in the
local raw archive under one durable transcript identity, then make bounded
archive completeness measurable without exposing private transcript data.

## User Contract

- Stop and `ingest-sessions` derive the same metadata-first identity. A
  transcript keeps one path-stable ledger ID when a filename fallback is later
  promoted to metadata.
- Batch discovery persists the complete identity claim set before raw mutation.
  Ambiguous claims remain sticky conflicts and fail visibly.
- Repeated identical turns at different JSONL ordinals remain distinct.
  Transcript timestamps, ingest fallbacks, and legacy-unknown event time remain
  distinguishable.
- `raw search`, `raw sessions`, and `raw reconcile` validate the current schema
  on a read-only connection. They never migrate or repair the store.
- Session JSON preserves existing fields and adds user/assistant role counts.
- `raw reconcile` requires an inclusive lower and upper bound, validates every
  captured file tuple against the current ledger, and compares stable
  per-occurrence identities.
- Reconciliation JSON is aggregate-only. It contains no path, project, session
  ID, hash, message text, or example.

## Parity

Parity requires exact transcript/archive occurrence multisets for the selected
window and no relevant identity conflict, malformed record, missing transcript
time, archive ingest-fallback time, or archive legacy-unknown time. Meta and
XML/control user records remain archived; their counts explain downstream
conversational exclusions without weakening raw completeness.

## Non-goals

- Reconciliation does not delete, repair, promote, or summarize messages.
- It does not make unbounded scans or network/LLM calls.
- This contract does not implement GH720's later refine/facet phases.
