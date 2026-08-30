# Refine Session Consumer Product Spec

Status: Current contract
Date: 2026-08-30

## Problem

Remem owns the raw transcript archive, but its existing `raw sessions` JSON
contract does not expose the originating coding-agent host or a session
fingerprint. A downstream cognitive-analysis consumer therefore has to fetch
every message before it can detect change, and cannot distinguish Codex CLI,
Claude Code, and Cursor without re-reading host-owned files.

That gap caused Refine to persist a second full transcript copy and to report
valid Codex observations as `platform_unknown`.

## Goals

1. Make Remem the only owner of complete coding-agent transcripts.
2. Expose trustworthy host provenance at the existing read-only raw-session
   boundary.
3. Let a bounded consumer skip unchanged sessions without fetching messages.
4. Give every session a stable opaque reference while preserving the exact
   selector tuple used by the raw-message API.
5. Fail explicitly when provenance is unavailable or ambiguous.

## Product Contract

`remem raw sessions --json` adds these fields to every session:

- `session_ref`: stable `remem://raw-session/v2/...` reference;
- `host`: `claude-code`, `codex-cli`, or `cursor`;
- `session_mode`: `interactive`, `unattended`, `subagent`, or `unknown`;
- `content_hash`: SHA-256 fingerprint of the ordered immutable raw
  occurrences selected for the session.

The existing `source_root`, `project`, and `session_id` fields remain the exact
raw-message selector. `source_root` identifies storage location; it is not a
host name.

`--latest N` returns at most `N` sessions ordered by newest message time. The
bound is applied by Remem before JSON is emitted so a scheduled consumer does
not enumerate an unbounded archive and truncate it afterward.

## Failure Semantics

- A session whose raw rows cannot be attributed to exactly one supported host
  is omitted from the successful contract and makes the command fail with an
  actionable error.
- Unbound `hook` fallback rows are partial capture evidence rather than complete
  transcripts. They remain available on raw occurrence/search surfaces but do
  not enter the Refine session contract or poison unrelated session listing.
- Two hosts must never collapse into one session summary, even when project and
  session ID are equal.
- Conflicting known session modes for one session fail explicitly. A missing or
  unrecognized upstream mode remains the honest `unknown` value.
- A malformed or zero `--latest` value is rejected.
- The command never falls back to a downstream filesystem scanner.

## Non-Goals

- No Refine-specific database tables in Remem.
- No direct access to Remem's SQLCipher database from Refine.
- No merger of Remem memory extraction and Refine cognitive-facet extraction.
- No new background service or configuration registry.

## Acceptance Criteria

- Claude Code, Codex CLI, and Cursor fixtures emit the exact host.
- Codex TUI/Desktop, automation, `codex_exec`/Symphony, and subagent fixtures
  emit their exact trusted session mode; other inputs emit `unknown`.
- Same project/session ID across two hosts yields two independently selectable
  summaries and references.
- Unchanged ordered occurrences yield an unchanged content hash; adding or
  rekeying an occurrence changes it.
- `--latest` is deterministic and bounded before serialization.
- CLI and MCP raw-session JSON keep one shared serialized shape.
- Refine can ingest a real Codex and Claude Code session without reading their
  transcript directories.
