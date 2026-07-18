# Product Spec

## Linked Issue

GH-871

## User Problem

The GH720 real-data acceptance run cannot currently establish trustworthy
parity between transcript files and remem's raw archive.

Two independent failures are visible:

- `remem raw search` and `remem raw sessions` open the database through the
  migration-capable read-write path. A normal writer holding the database can
  therefore make a read-only query fail with `database is locked`.
- Transcript files, Stop-hook captures, and historical raw rows do not expose
  one durable, auditable session identity contract. Aggregate totals cannot
  distinguish identity drift from intentionally excluded meta/XML records or
  genuinely missing archive rows.

Without a deterministic and privacy-safe reconciliation surface, operators
cannot tell whether a mismatch is data loss, an identity alias, or an expected
inclusion-rule difference.

## Goals

- Make both raw CLI query surfaces true read-only operations that remain
  available during normal database write contention.
- Define and persist one canonical mapping from a transcript file identity to
  the raw archive session identity.
- Reconcile already-ingested filename-derived rows without losing messages or
  silently choosing between conflicting mappings.
- Provide a deterministic fixed-window reconciliation report that explains
  every count difference without emitting message text or private identifiers.
- Produce sufficient evidence to complete GH720 task `SP720-T4`.

## Non-Goals

- Change curated-memory promotion, ranking, or retention behavior.
- Delete raw messages merely because a recap-style consumer excludes them.
- Copy the recap script's abbreviated session IDs or display-only project
  names into remem's durable identity.
- Emit transcript paths, project names, full session IDs, message text, or
  content hashes in reconciliation output.
- Implement GH720 Phase 2 refine migration or Phase 3 facet extraction.
- Reconcile any issue other than GH-871 in this implementation tranche.

## Behavior Invariants

1. `remem raw search` and `remem raw sessions` open an existing current
   database read-only, perform no schema migration or other write, and succeed
   while another normal remem connection holds a write transaction.
2. A transcript-provided session ID is the canonical raw archive session ID.
   The filename stem is used only when the supported transcript format has no
   session ID, and that fallback provenance remains auditable.
3. Each discovered transcript has one durable local mapping containing its
   source root, canonical session ID, project identity, identity source, and a
   privacy-safe path fingerprint. The public reconciliation report never
   exposes the underlying path or identifiers.
4. Batch ingestion refreshes the durable mapping even when the transcript's
   mtime and size cursor is unchanged. A file first ingested under a filename
   fallback can therefore converge after canonical metadata support is added.
5. When one unambiguous filename-derived legacy identity maps to a canonical
   identity, ingestion rekeys its raw rows transactionally. Rows that collide
   with an already-canonical copy are merged by the existing raw-message
   identity, not duplicated or dropped. Repeating the reconciliation is
   idempotent.
6. Ambiguous, conflicting, missing, or unsafe legacy mappings are never
   guessed. Their raw rows remain unchanged, the ingestion run returns an
   error-level diagnostic, and the reconciliation report counts the conflict.
7. The session query contract reports `message_count`, `user_message_count`,
   and `assistant_message_count` for each session. Existing fields and grouping
   by `(source_root, project, session_id)` remain compatible.
8. Reconciliation applies one versioned transcript inclusion policy. Archive
   eligibility and intentional exclusions are counted separately for:
   conversational user messages, assistant messages, meta user messages,
   XML/control user messages, empty text, unsupported records, records outside
   the fixed UTC window, and malformed records.
9. Raw capture remains lossless for supported non-empty text. A record excluded
   from the reconciliation's conversational-user count is not deleted from the
   archive.
10. `remem raw reconcile --since <epoch> --until <epoch> --json` uses inclusive
    UTC bounds and produces aggregate counts for exact matches, transcript-only
    sessions/messages, archive-only sessions/messages, identity conflicts, and
    every intentional exclusion category.
11. Reconciliation output is deterministic for an unchanged database and
    transcript set. It emits no message text, samples, transcript paths, project
    names, full session IDs, or content hashes.
12. An empty transcript set, empty archive window, missing optional default
    root, or zero mismatches produces a successful explicit zero-count report.
    Missing required roots, database read failures, malformed bounds, and
    identity conflicts fail loudly.

## Acceptance Criteria

- [ ] A lock-contention regression proves both raw CLI query surfaces succeed
      while a separate connection holds a normal write transaction.
- [ ] Automated tests prove metadata-first identity, filename fallback
      provenance, unchanged-cursor mapping refresh, conflict-safe legacy rekey,
      collision merge, and idempotent rerun behavior.
- [ ] Raw session JSON exposes total/user/assistant counts with compatibility
      tests for existing fields and grouping.
- [ ] Reconciliation fixtures cover exact parity, transcript-only,
      archive-only, conflicting identity, meta/XML exclusion, malformed record,
      and fixed-window boundary cases without sensitive output.
- [ ] The recorded GH-871 UTC window is rerun with the privacy-safe command and
      reaches parity or reports a non-zero count for every remaining difference
      category.
- [ ] `specs/GH720/tasks.md`, the current raw-session ingestion contract,
      `README.md`, and `docs/ARCHITECTURE.md` describe the shipped behavior.

## Edge Cases

- A transcript is actively appended while reconciliation reads it. Only
  complete JSONL records observed within the captured file boundary count.
- A Stop hook has already inserted canonical rows and an older batch pass
  inserted the same messages under a filename fallback. Backfill converges to
  one canonical session without losing unique messages.
- Two transcript paths claim the same fallback identity but different
  canonical IDs. Automatic rekey is refused and the conflict is counted.
- A historical raw row has no discoverable transcript. It remains archive-only
  and is not rewritten.
- A user record is meta, begins with XML/control markup, or contains no
  supported text. The report assigns exactly one exclusion category.
- The same session spans the UTC window boundary. Only records whose transcript
  event time is inside the inclusive window participate in parity counts.

## Rollout Notes

The schema addition and backfill are additive. Identity convergence runs from
the existing explicit `ingest-sessions` write surface; raw query and
reconciliation commands remain read-only. Operators should run
`ingest-sessions` once before the fixed-window reconciliation so unchanged
historical cursors receive the mapping refresh. Conflicts remain preserved for
manual inspection rather than being silently rewritten.
