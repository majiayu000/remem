# Session Intent Display Product Spec

Status: Current contract (Phase 1 schema/display landed as v092; Phase 2 summary persist in #1067; #1068–#1069 remaining)
Date: 2026-09-05

Tracking:
- Capability epic: #1065
- Implementation: #1066 (schema/display), #1067 (summary), #1068 (UI/governance), #1069 (candidates/docs)
- Related: #603 (workstream identity continuity), `session-observatory/`, `refine-session-consumer/`

## Problem

Coding-agent session lists are hard to scan. Operators often pack
`MMDD｜TYPE｜Topic` into host conversation titles so the sidebar is readable.
That workaround is host-local, encourages title-as-identity (which splits
workstreams under rename drift), and sits outside Remem’s product boundary.

Remem already stores raw sessions, summaries, typed memories, and workstreams,
but it lacks a first-class **conversation intent** layer and a shared display
label. Memory types answer “what should be remembered?”; they must not be
overloaded to mean “what was this session for?”

## Goal

Make sessions and workstreams scannable inside Remem by:

1. Storing optional structured `session_intent` and `session_topic`.
2. Deriving a local `MMDD` date from **created** time in `Asia/Shanghai`.
3. Rendering a stable label `{MMDD}｜{INTENT}｜{topic}` in CLI, API, and app.
4. Abstaining when intent or topic cannot be determined.
5. Keeping host conversation titles immutable from Remem.
6. Keeping workstream identity stable across display/intent updates (#603).

## User Stories

### Scan today’s sessions

As a user reviewing recent Codex/Claude work, I want each summarized session to
show a short label such as `0903｜FIX｜Batch text display` so I can find the
right thread without opening transcripts.

### Abstain rather than guess

As a user, when Remem cannot tell what a session was for, I want the original
request/title left alone and the intent left empty—not a fabricated TYPE.

### Correct a bad auto label

As a maintainer, I want a preview table of Before/After intent/topic changes
before any batch override applies, and I want that override audited.

### Rename without splitting workstreams

As a user, when the display topic of a long-running task changes, Remem should
update the canonical workstream’s display fields and aliases, not create a
second active workstream.

## Product Contract

### Layer separation

| Layer | Field / vocabulary | Role |
|---|---|---|
| Session / workstream intent | `session_intent` closed enum | Why this conversation/task existed |
| Observation type | existing observation vocabulary | What a summary/turn extracted |
| Memory type | existing `MemoryType` | What should be recalled later |

Display may document a mapping table between layers. Storage and retrieval
filters must not treat them as the same field.

### Closed intent vocabulary (v1)

English codes are canonical in storage and default display:

| Code | Meaning |
|---|---|
| `fea` | feature |
| `des` | design |
| `fix` | bug fix |
| `opt` | optimization |
| `rel` | release |
| `exp` | exploration |
| `doc` | docs |
| `res` | research |

Optional Chinese display labels may be rendered on request, but a single run of
UI/CLI output must not mix languages for the INTENT segment.

Unknown codes are rejected at write time (fail closed) or treated as abstain on
untrusted model output.

### Topic

- Short, specific, sidebar-friendly.
- Must not repeat the project name.
- Must not replace raw transcript content.
- If unknown, keep prior topic/null; do not invent.

### Date segment

- Source: session or workstream **created** epoch.
- Zone: `Asia/Shanghai`.
- Format: `MMDD`.
- Never use `updated_at` / last activity for the label date.

### Display label

```text
{MMDD}｜{INTENT}｜{topic}
```

Rules:

- Render the full label only when both INTENT and topic are present.
- Otherwise show an explicit abstain/empty intent state plus the best available
  fallback title (`request`, workstream title, or host-provided title sample)
  without fabricating INTENT.
- Remem never writes this string back into Codex/Claude conversation titles.

### Write paths

1. **Auto (summary):** optional extraction; abstain on low confidence.
2. **Manual override:** preview-then-apply; audited.
3. **Display-only derivation:** `MMDD` is computed, not a second source of truth.

Auto extraction must not block memory promotion when intent/topic are absent.

### Workstream identity

Intent/topic/display updates are mutable attributes of the canonical
workstream. They:

- may add aliases for prior topics/titles;
- must not create a new workstream row solely because the label changed;
- must remain compatible with #603 matching order and SessionStart canonical
  rendering.

## Non-Goals

- Batch-renaming host agent conversation titles.
- Merging tip TYPE codes into `memory_type`.
- Keepline ownership of session identity or history search.
- Auto-closing workstreams from GitHub/PR state.
- Requiring agents to call a new MCP tool for ordinary continuity.

## Success Metrics

| Metric | Target |
|---|---|
| Label coverage on summarized sessions | Most rows show a label or explicit abstain |
| Wrong-intent rate | Spot checks prefer abstain over incorrect TYPE |
| Rename-induced workstream splits | No new splits from intent/display updates |
| Host title mutation | Zero Remem write paths to host conversation titles |

## Acceptance Criteria

- Spec pair and index entry land under `docs/specs/session-intent-display/`.
- Implementation issues #1066–#1069 cover schema/display, summary, UI/governance,
  and candidate/docs phases.
- Epic #1065 closes only after those phases are verified, not when this spec
  merges.

## Open Questions

1. Should workstreams store their own intent, inherit the latest linked session
   intent, or both?
   Recommendation: store on both; session intent is authoritative for that
   session, workstream intent is the latest high-confidence rollup with audit.
2. Should `raw sessions` JSON include computed label fields before a summary
   exists?
   Recommendation: yes for `mmdd` from created epoch; intent/topic remain null
   until summary/override.
3. Should prompt-time candidates show INTENT badges by default?
   Recommendation: optional and compact; default off until Phase 4 measures
   noise.
