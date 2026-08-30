# Refine Session Consumer Technical Spec

Status: Current contract
Date: 2026-08-30

## Existing Boundaries

- Canonical raw occurrences live in `raw_messages`.
- Identified occurrences reference `raw_session_identities`. Migration v091
  adds the authoritative nullable `host` and closed `session_mode`; trusted
  ingestion writes them together with the transcript path.
- `raw sessions` and the MCP `list_raw_sessions` tool already share
  `RawSessionSummary`.
- `raw messages` already requires the exact `source_root + project +
  session_id` tuple and uses a validated cursor.

## Host Resolution

Resolve host inside Remem from every contributing active transcript identity:

| Transcript identity path | Host |
| --- | --- |
| under `.claude/projects` | `claude-code` |
| under `.codex/sessions` | `codex-cli` |

New ingestion resolves with path components, not substring matching. Migration
v091 backfills only unambiguous legacy path shapes. Every raw
occurrence in a returned session must have an identified transcript and all
identities must resolve to the same host. Legacy unidentified rows and mixed
hosts fail closed. Unbound `source=hook` fallbacks are explicitly outside this
complete-transcript surface and remain queryable through the existing raw
occurrence/search paths. The host rules remain in Remem; consumers receive only
the resolved value.

Cursor does not enter this transcript-identity path. A virtual Cursor
outcome-only session can expose `host: cursor` plus `capture_health`, but it has
no complete raw-message selector and Refine must not ingest it as a transcript.

## Session Identity

The grouping key becomes:

```text
(source_root, host, project, session_id)
```

`session_ref` is a deterministic versioned encoding of that tuple. It is an
opaque reference for consumers, not a filesystem path and not authorization.
Raw messages continue to be selected using the explicit tuple fields supplied
in the summary; Remem validates that the selected rows resolve back to the
same host.

## Session Mode

For Codex transcripts, Remem reads `session_meta.payload.thread_source` and
`originator` at the existing transcript probe boundary. `subagent` has highest
precedence, followed by unattended originators (`codex_exec` and
`symphony-orchestrator`) or the explicit `automation` thread source, then
interactive TUI/Desktop originators. Unknown values remain `unknown`; Claude
Code currently reports `unknown`.
An identity may promote from unknown to a known mode, but conflicting known
modes fail before mutation. A grouped raw session also fails if contributing
identities disagree.

## Content Fingerprint

For the selected window, order occurrences by:

```text
created_at_epoch ASC, raw_message.id ASC
```

Hash a version prefix plus each occurrence's stable identity, role,
`content_hash`, and event time using length-delimited fields. The result is
formatted as `sha256:<hex>`. Samples do not affect the fingerprint.

This computation is transport metadata only. Raw content remains canonical in
`raw_messages` and is never copied into a new Remem table.

## Bounded Listing

Extend `RawSessionQuery` with `latest: Option<i64>`. Without `latest`, retain
the existing window ordering. With it, group the selected raw occurrences,
order by `last_epoch DESC` and the full grouping tuple, then truncate before
serialization. The JSON envelope records the requested bound. This bounds the
consumer transport and full-message fetches; it does not claim an SQL-level
metadata scan limit.

## Consumer Contract

Refine first lists bounded summaries. It compares `session_ref + content_hash +
session_mode` with its local projection. Only new or changed sessions
invoke paginated `raw messages`. Each page is bound to the exact `host` and
returns the fingerprint of its frozen cursor snapshot; Refine must reject the
export if that fingerprint differs from the selected summary or changes between
pages. Refine may hold the reconstructed transcript
in memory for extraction but must persist only the reference, fingerprint,
host, timestamps, and derived observations.

## Tests

- component-safe host classification and near-miss paths;
- Claude Code and Codex CLI transcript host fixtures;
- Cursor outcome-only metadata is not accepted as a complete transcript;
- mixed/unidentified identity rejection;
- same session ID collision isolation across hosts;
- deterministic fingerprint and mutation sensitivity;
- deterministic `--latest` ordering and argument validation;
- CLI/MCP JSON schema updates;
- real binary contract smoke consumed by Refine.
