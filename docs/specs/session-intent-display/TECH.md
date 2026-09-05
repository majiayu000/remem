# Session Intent Display Technical Spec

Status: Current contract (Phase 1 schema/display landed as migration v092; #1067–#1069 remaining)
Date: 2026-09-05

Tracking:
- Capability epic: #1065
- Implementation: #1066, #1067, #1068, #1069
- Related: #603, `session-observatory/`, `refine-session-consumer/`, `raw-session-ingestion/`

## Existing Implementation Facts

- Raw sessions are listed via `remem raw sessions` / MCP `list_raw_sessions` with
  host-bound identity, epochs, role counts, and optional `capture_health`
  (`refine-session-consumer/`, `raw-session-ingestion/`).
- Semantic session fields live on `session_summaries` as `request`, `completed`,
  `decisions`, `learned`, `next_steps`, and related metadata.
- Memories use closed `MemoryType` values in `src/memory/types.rs`; observation
  types are a different vocabulary mapped through
  `MemoryType::from_observation_type`.
- Workstreams persist free-text `title` plus status/next/blockers; #603 owns
  identity continuity across title drift via aliases/session links.
- Session Observatory projects turn activity without replacing summaries.
- Summary persistence already upserts workstreams from LLM title strings;
  display-oriented renames must not strengthen title-as-identity.

## Design Principles

1. **Additive schema.** No rewrite of raw archive rows.
2. **Derived date.** `MMDD` is computed from created epoch + `Asia/Shanghai`.
3. **Fail closed on writes, abstain on model output.** Unknown stored values
   reject; untrusted summary fields become null.
4. **No host mutation.** No adapter writes Codex/Claude conversation titles.
5. **Identity ≠ display.** Workstream matching continues to follow #603.

## Storage Contract

Additive migration `v092_session_intent_display`:

```sql
ALTER TABLE session_summaries ADD COLUMN session_intent TEXT;
ALTER TABLE session_summaries ADD COLUMN session_topic TEXT;
ALTER TABLE session_summaries ADD COLUMN session_intent_source TEXT;
ALTER TABLE session_summaries ADD COLUMN session_intent_updated_at_epoch INTEGER;

ALTER TABLE workstreams ADD COLUMN session_intent TEXT;
ALTER TABLE workstreams ADD COLUMN session_topic TEXT;
ALTER TABLE workstreams ADD COLUMN session_intent_source TEXT;
ALTER TABLE workstreams ADD COLUMN session_intent_updated_at_epoch INTEGER;
```

Closed values:

- `session_intent`: `fea` | `des` | `fix` | `opt` | `rel` | `exp` | `doc` |
  `res` | NULL
- `session_intent_source`: `summary` | `override` | `rollup` | NULL

Constraints:

- CHECK constraints (or equivalent fail-closed writers) reject unknown intent
  codes and unknown sources.
- `session_topic` is nullable, trimmed, length-bounded (recommended ≤ 80
  Unicode chars after trim), and passed through the shared sensitive-text
  redactor before persistence.
- NULL intent or NULL topic means abstain for label rendering.

Do not store the rendered label string as a primary column. Render at read
time so timezone/format policy can evolve without backfills.

## Display Helper

Pure helper `src/memory/session_label.rs` (`pub(crate)` under `memory`, not a
crate-root module):

```text
inputs:
  created_at_epoch: i64
  intent: Option<SessionIntent>
  topic: Option<&str>
  fallback_title: Option<&str>
  intent_language: English | Chinese  # default English

outputs:
  mmdd: String                     # always from created_at_epoch
  label: Option<String>            # Some only when intent+topic present
  display_title: String            # label or fallback/abstain rendering
```

Rules:

- Timezone fixed to `Asia/Shanghai` for v1.
- INTENT segment uses English codes by default; Chinese labels only when
  explicitly requested for a whole response.
- Full-width separator `｜` is required for parity with the operator tip.
- Never invent topic from project path basename.

## Read Surfaces

### `raw sessions` / Refine consumer JSON

Additive nullable fields on each session summary object:

| Field | Meaning |
|---|---|
| `mmdd` | Derived from `first_epoch` / created epoch |
| `session_intent` | Closed code or null |
| `session_topic` | Short topic or null |
| `display_label` | Full label or null |
| `session_intent_source` | Provenance or null |

Existing fields remain compatible. Missing summary ⇒ intent/topic null, `mmdd`
still derived when created epoch exists.

### REST / app session list

Same additive fields on Session Observatory / sessions list payloads. Label
column renders `display_label` or an abstain placeholder plus fallback title.

### Workstreams MCP/CLI

Expose the same additive fields on workstream rows. `update_workstream` may set
intent/topic only through an explicit override path with validation; ordinary
progress updates must not clear intent unless requested.

## Write Surfaces

### Summary extraction (#1067)

Extend `prompts/summary.txt` with optional:

```xml
<session_intent>fix</session_intent>
<session_topic>Batch text display</session_topic>
```

Parse rules:

- Unknown/empty intent ⇒ NULL (abstain), do not fail the summary job.
- Topic empty/too long/redacted-empty ⇒ NULL.
- On success, set `session_intent_source = 'summary'`.
- Memory candidate promotion remains independent.

### Manual override (#1068)

CLI/API preview-then-apply:

1. Operator submits target session/workstream IDs + proposed intent/topic.
2. Response is a two-column table contract: `Before` / `After` (and stable ids).
3. Apply only after explicit confirmation token/flag.
4. Persist `session_intent_source = 'override'` and updated epoch.
5. For workstreams, record prior topic/title through #603 alias machinery when
   the display topic changes.

### Workstream rollup

Optional later: when a linked session gains high-confidence intent and the
workstream intent is null, copy with `session_intent_source = 'rollup'`.
Do not overwrite override-sourced values automatically.

## Explicit Non-Paths

- No code path calls host APIs to rename conversations.
- No migration backfills guessed intents for historical rows.
- No `memory_type` aliasing to `session_intent`.
- No Keepline dual-write.

## Mapping Table (documentation only)

| Intent | Common observation | Common memory |
|---|---|---|
| `fix` | bugfix | bugfix |
| `fea` | feature | discovery / architecture |
| `des` | change / feature | architecture / decision |
| `opt` | refactor | discovery / decision |
| `doc` | discovery | discovery |
| `exp` | discovery | discovery |
| `rel` | change | decision |
| `res` | discovery | discovery / architecture |

## Test Plan

| Case | Expectation |
|---|---|
| Created epoch → Asia/Shanghai MMDD | Stable across process TZ |
| Intent+topic present | `display_label` formatted with `｜` |
| Missing intent or topic | `display_label` null; fallback title used |
| Invalid intent on override | Rejected |
| Invalid intent from summary | Abstain (null), summary still persists |
| Topic redaction empties string | Abstain topic |
| Workstream topic change | Alias preserved; no new identity row |
| Raw sessions without summary | `mmdd` only |
| Host rename | No Remem writer exists (lint/test guard if feasible) |

## Rollout

1. Spec merge (this PR) with `Refs #1065`.
2. #1066 schema + helpers + list fields.
3. #1067 summary abstain path.
4. #1068 UI + preview governance.
5. #1069 candidate title polish + host-tip docs boundary.

Epic #1065 closes only after phases are verified.

## Open Implementation Notes

- Shared Rust enum `SessionIntent` lives in `src/memory/session_label.rs` so
  MCP/REST/CLI renderers cannot drift. Writers in #1067/#1068 must reuse
  `parse_write`.
- Chinese labels stay display-only; storage and default English rendering use
  closed codes (`fix`, not `FIX`).
- Session Observatory projection tables remain a possible later attachment
  point for list performance; summary is still the semantic write source.
