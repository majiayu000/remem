# Prompt-Time Candidate Index Technical Spec

Status: Current contract
Date: 2026-09-02

## Existing Pieces Reused

- `src/observe/session_init.rs` already parses and captures
  UserPromptSubmit and emits the Codex/Claude `additionalContext` envelope.
- `src/context/prompt_submit_retrieval.rs` already calls the bounded hybrid
  retrieval path and its weighted RRF fusion.
- `src/context/query.rs`, `src/context/summary_query.rs`, and
  `src/context/poisoning.rs` already own project/owner filtering and safe
  SessionStart continuity inputs.
- `context_injection_items` already records prompt-time selection and drop
  evidence.

## Runtime Flow

```text
UserPromptSubmit
  -> capture user prompt
  -> count prompt events for this host/session
  -> first event only: load safe continuity anchors (max 2)
  -> hybrid memory retrieval (max 4)
  -> current-truth + poisoning + already-injected filters
  -> render compact candidate metadata
  -> hookSpecificOutput.additionalContext
```

The captured-event count is session state, not an intent heuristic. Codex uses
its native `turn_id` to make a retry of the same UserPromptSubmit event
idempotent. The prompt event id also scopes the existing injection audit key:
same-turn retries exclude their own prior audit rows and deterministically
replay the same candidates, while later turns retain session-level de-duplication.
Claude does not provide a turn id, so each hook occurrence receives a unique
capture event id; repeated identical prompt text therefore still advances the
session prompt count. The continuity lane is omitted once the session contains
more than one distinct prompt event.

## Candidate Rendering

Memory candidates contain:

- `memory:#<id>`
- memory type and title
- stable updated date and staleness/source-anchor metadata
- `surfaced_by=hybrid_rrf`
- approximate detail read tokens derived from the complete pretty JSON returned
  by the advertised exact reader
- exact `open=get_observations source=memory ids=[...]`

Continuity candidates contain the stable workstream or session-summary id,
compact title/request, state or next action, stable updated date,
`surfaced_by=first_turn_continuity`, approximate detail read cost, and a
project-scoped active `workstreams` lookup hint or exact
`get_observations(source=session_summary, ids=[...])` call.

Each read cost matches its advertised reader: memory cost covers the complete
exact-detail JSON, including classification metadata, temporal facts, and topic
trace when present; workstream cost covers the complete active-list JSON for
the advertised project/status lookup; session-summary cost covers the complete
serialized exact-detail response, including all six summary text fields and
its metadata. Estimating memory detail reuses the reader's side-effect-free
detail builder and does not mark the memory as accessed.

All free text is normalized to one line and truncated at an item boundary. The
renderer remains an additive prompt block and never rewrites SessionStart.

## Filtering And Audit

- Memory candidates retain current G2/current-truth, owner/scope, lifecycle,
  suppression, source-anchor, and poisoning checks. Temporal fact annotation
  runs once per bounded retrieval batch, and poisoning scans every rendered
  field, including `memory_type`, before admission.
- Workstream and session anchors use owner-aware filters and bounded backfill
  after poisoning admission. Workstream scans fail explicitly if their bounded
  budget cannot find enough safe anchors. The single session-anchor scan reads
  at most 200 rows plus one lookahead row and reports exhaustion only when more
  eligible rows remain. An unfinished row may derive its compact title from
  `next_steps` when `request` is absent. Session anchors scan `request`,
  `completed`, `decisions`, `learned`, `next_steps`, and `preferences`, matching
  their exact detail reader before any summary text enters the prompt. Summary
  selection applies the same seconds-only epoch filter as that exact reader.
- Already injected memories remain excluded for the same host/project/session.
- Canonical project summary selection and exact detail reads share the same
  repo `owner_key` / repo `target_project` / legacy `project` predicate and
  include active historical aliases.
- The four-item memory limit is applied only after G2, already-injected, and
  poisoning admission, with a bounded ranked scan to backfill safe candidates.
- RRF rank is a candidate ordering signal only. There is no final relevance
  threshold.
- Only anchors and memories whose complete lines fit the 1,800-character
  prompt block are `injected` audit items. Item-boundary omissions keep a
  stable character-limit drop reason; rendered memory types are bounded. Other
  rejected rows keep their existing reasons, and an empty result after actual
  line rendering records an abstention instead of returning a header-only
  block.

## Files

- `src/install/config.rs`, `src/install/hosts/codex.rs`
- `src/context/host.rs`
- `src/context/prompt_submit.rs`
- `src/context/poisoning.rs`
- `src/db/query/summaries.rs`
- `src/mcp/types.rs`, `src/mcp/server/context_tools.rs`, and the existing
  `get_observations` output contract
- `src/observe/session_init.rs`
- README, architecture, plugin activation docs, changelog, and synchronized
  package versions

## Verification

- Focused Rust tests for Codex hook generation, host capability,
  UserPromptSubmit parsing/capture, candidate rendering, first-turn continuity,
  filtering, determinism, and latency.
- Injection evaluation requires the unrelated-prompt fixture to return no
  additional context before its abstention metric can pass.
- Version-sync and JavaScript plugin/npm tests.
- `cargo fmt --check`, `cargo check`, and `cargo test`.
- Isolated HOME/REMEM_DATA_DIR install, context, status, doctor, and plugin
  activation smoke.
