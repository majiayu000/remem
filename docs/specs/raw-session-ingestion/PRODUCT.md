# Raw Session Ingestion

Status: Current contract
Refs: GH-871, GH-720, GH-825

## Goal

Preserve every supported Claude Code, Codex, and approved Cursor transcript
occurrence in the local raw archive under one durable transcript identity,
then make bounded archive completeness measurable without exposing private
transcript data.

## User Contract

- Stop and `ingest-sessions` derive the same metadata-first identity. A
  transcript keeps one path-stable ledger ID when a filename fallback is later
  promoted to metadata.
- Batch discovery persists the complete identity claim set before raw mutation.
  Ambiguous claims remain sticky conflicts and fail visibly.
- Repeated identical turns at different JSONL ordinals remain distinct.
  Transcript timestamps, ingest fallbacks, and legacy-unknown event time remain
  distinguishable.
- GH-825 Cursor full snapshots preserve every approved message occurrence by
  transcript identity plus zero-based physical JSONL record ordinal. Internal
  non-message records may leave ordinal gaps. Leading/trailing whitespace and
  complete UTF-8 text bytes are stable evidence: the Cursor path never calls a
  legacy occurrence helper that trims content before hashing or storage.
- An unchanged Cursor identity+ordinal+stable-field replay is a no-op; changed
  whitespace, role, text, or other stable evidence is a visible identity
  conflict that rolls back the whole snapshot bundle. Identified Cursor rows
  never fall back to content-only deduplication.
- `raw search`, `raw sessions`, and `raw reconcile` validate the current schema
  on a read-only connection. They never migrate or repair the store.
- Session JSON preserves existing fields and adds user/assistant role counts.
- Session JSON also preserves every existing field and adds one nullable
  `capture_health` field at the session level. `raw sessions --json` builds its
  candidate set as the union of raw-message-backed sessions and authoritative
  Cursor terminal outcomes, so a degraded/blank outcome with no raw message or
  summary remains visible. An outcome-only row uses the reserved transport
  locator `source_root: "cursor-outcome"`, the source Stop epoch for
  `first_epoch`/`last_epoch`, zero message/role counts, and empty samples; it is
  not a persisted raw source root and must not collide with or fabricate raw
  messages. In `raw messages --json`, selecting that exact locator returns an
  empty message page plus the same non-null `capture_health` envelope.
  `cursor-outcome` is reserved: user `--root cursor-outcome=...` input and a
  required ingest root constructed around the parser are rejected before
  persistence, while default internal roots remain unchanged. A persisted raw
  or identity row using that label is an explicit read collision, not a row to
  merge with the virtual locator.
  Individual message objects never carry the field. For the
  authoritative current Cursor outcome it is an object with
  `fidelity` (`full`, `degraded`, or `blank`), the approved Stop `status`, a
  stable nullable `reason_code`, and an opaque redacted `stop_key`. `full`
  requires `reason_code: null`; `blank` requires
  `reason_code: "no_usable_evidence"`, an empty result, and no summary ID or
  range. A non-Cursor session or a Cursor session with no terminal outcome
  returns `capture_health: null`. Malformed or unreadable outcome state is an
  explicit command error, not `null`, `full`, or an empty successful result.
  Session-level authority is selected in two stages: first choose the current
  source Stop by its immutable capture order
  `(captured_events.created_at_epoch, captured_events.id)`, then apply
  `full > degraded > blank` only within that Stop. Outcome replay references
  the original Stop and never advances this order, so an earlier full result
  cannot mask a later degraded or blank Stop. Doctor retains every Stop for
  history and aggregate counts.
- `raw reconcile` requires an inclusive lower and upper bound, validates every
  captured file tuple against the current ledger, and compares stable
  per-occurrence identities.
- For a Cursor identity, `raw reconcile` orders all authoritative full
  companions by approved snapshot length, verifies one monotonic prefix chain,
  and selects the longest approved boundary before reusing the same versioned
  record/physical-ordinal projection as Stop capture. It never sends Cursor
  `{role,message}` records through the Claude/Codex classifier. A fork,
  truncation, mutated prefix, or missing/malformed/conflicting parser metadata
  is a visible reconciliation error, not an unsupported-record exclusion or
  guessed fallback.
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
- GH-825 does not add raw archive tables, columns, indexes, or a schema version;
  it adds a versioned Cursor parser/projection and an exact-content insertion
  path over the existing identified-occurrence schema.
