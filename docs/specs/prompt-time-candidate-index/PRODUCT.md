# Prompt-Time Candidate Index Product Spec

Status: Current contract
Date: 2026-09-02

## Problem

The existing UserPromptSubmit path renders memory body previews and then applies
a lexical-overlap threshold after hybrid retrieval. The extra threshold can
discard useful semantic or continuity candidates, while body previews make a
retrieval lead look like a decision the model must use. Codex supports
UserPromptSubmit but remem does not currently install that hook for Codex.

## Goals

- Install the existing `session-init --host codex-cli` runtime on Codex
  UserPromptSubmit.
- Give the model a compact, optional candidate index on each prompt instead of
  preloading memory bodies.
- Preserve model autonomy: retrieval ranks possible leads; the model decides
  whether to ignore, open, search beyond, or use them.
- Make a first-turn prompt such as `continue` useful through bounded continuity
  anchors that do not depend on lexical overlap.
- Keep every surfaced or dropped candidate auditable.

## Non-Goals

- A confidence threshold that decides relevance for the model.
- A second LLM reranker or intent classifier.
- New retrieval weights, configuration, schema, or background work.
- Replacing SessionStart or Stop capture.
- Injecting full memory, session, or workstream bodies at prompt time.

## Behavior

1. Codex install and explicit plugin activation write SessionStart,
   UserPromptSubmit, and Stop hooks.
2. The first UserPromptSubmit in a session may surface at most two continuity
   anchors: the most recent safe active workstream and the most recent safe
   session with unfinished next steps. If one kind is absent, another safe
   active workstream may fill the remaining slot.
3. Every non-empty prompt runs the existing project-scoped hybrid retrieval and
   may surface at most four memory candidates after current-truth,
   suppression, ownership, lifecycle, poisoning, and per-session duplicate
   filters.
4. No post-retrieval lexical or confidence threshold decides whether a
   surviving candidate is relevant.
5. The rendered block contains identifiers, kind/title, update state, why it
   was surfaced, approximate read cost, and an exact detail-tool call. Session
   anchors use `get_observations(source=session_summary, ids=[...])` rather
   than reusing the unrelated observation timeline ID namespace. The block
   does not contain memory bodies.
6. The block states that candidates are optional leads, not instructions or
   established relevance. Memory citations apply only after a memory is opened
   and actually used.
7. If no candidate survives, the hook emits no additional context and records
   an abstention audit row.

## Acceptance Criteria

- Codex hook tests require the UserPromptSubmit command and timeout.
- A semantic/hybrid hit is surfaced even when the old lexical-overlap gate
  would have rejected it.
- Candidate output excludes stored memory body text and includes an explicit
  detail lookup hint.
- Only candidate lines that fit the 1,800-character block are audited as
  injected; candidates omitted at an item boundary receive a drop audit.
- A first-turn `continue` prompt can surface continuity anchors; a later prompt
  in the same captured session does not repeat that lane.
- Existing current-truth, poisoning, duplicate, determinism, latency, and
  additive-layering checks stay green.
