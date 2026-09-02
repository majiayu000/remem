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
idempotent. The continuity lane is omitted once the session contains more than
one distinct prompt event.

## Candidate Rendering

Memory candidates contain:

- `memory:#<id>`
- memory type and title
- stable updated date and staleness/source-anchor metadata
- `surfaced_by=hybrid_rrf`
- approximate detail read tokens derived from stored character count
- `open=get_observations`

Continuity candidates contain the stable workstream or session-summary id,
compact title/request, state or next action, stable updated date,
`surfaced_by=first_turn_continuity`, approximate detail read cost, and a
`workstreams` or `timeline` lookup hint.

All free text is normalized to one line and truncated at an item boundary. The
renderer remains an additive prompt block and never rewrites SessionStart.

## Filtering And Audit

- Memory candidates retain current G2/current-truth, owner/scope, lifecycle,
  suppression, source-anchor, and poisoning checks.
- Workstream and session anchors use the existing owner-aware and poisoning
  filters before their text can enter the prompt.
- Already injected memories remain excluded for the same host/project/session.
- RRF rank is a candidate ordering signal only. There is no final relevance
  threshold.
- Surfaced anchors and memories are `injected` audit items; rejected rows keep
  a stable drop reason; an empty result records an abstention.

## Files

- `src/install/config.rs`, `src/install/hosts/codex.rs`
- `src/context/host.rs`
- `src/context/prompt_submit.rs`
- `src/context/poisoning.rs`
- `src/observe/session_init.rs`
- README, architecture, plugin activation docs, changelog, and synchronized
  package versions

## Verification

- Focused Rust tests for Codex hook generation, host capability,
  UserPromptSubmit parsing/capture, candidate rendering, first-turn continuity,
  filtering, determinism, and latency.
- Version-sync and JavaScript plugin/npm tests.
- `cargo fmt --check`, `cargo check`, and `cargo test`.
- Isolated HOME/REMEM_DATA_DIR install, context, status, doctor, and plugin
  activation smoke.
