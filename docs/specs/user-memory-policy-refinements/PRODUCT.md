# User Memory Policy Refinements Product Spec

Status: Proposed current contract
Date: 2026-06-24

Tracking:
- Spec/tracking issue: #617
- Profile Markdown snapshot: #619
- Usage policy injection rules: #620
- Automatic extraction blocklist: #618

## Problem

The user-context layer already gives remem an engineering-grade memory model:
claims, summaries, suppressions, feedback, source refs, review-gated candidates,
and owner-aware recall. That is the right source-of-truth model for a multi-owner
coding-agent memory system.

The remaining product gap is not storage strength. It is inspectability and
social comfort:

- users should be able to see a compact "what remem believes about me" snapshot
  without reading SQLite rows;
- agents should use user context naturally without repeatedly saying they are
  using memory;
- automatic extraction should have explicit non-retention rules for common
  profile-pollution cases.

This spec refines user memory policy without replacing the existing
`user_context_*` schema.

## Goals

- Add a derived, human-readable user profile snapshot that can be reviewed and
  versioned outside SQLite.
- Make usage rules explicit for injected and recalled user context.
- Harden automatic extraction with a concrete "do not retain" blocklist.
- Preserve the current source-of-truth architecture: SQLite claims, candidates,
  summaries, suppressions, feedback, and source refs remain canonical.
- Keep changes phased so each piece can ship with focused tests.

## Non-Goals

- No migration from SQLite to a single `memory.md` file.
- No hidden profile synthesis that cannot be traced to source ids.
- No automatic promotion of personal, sensitive, restricted, speculative, or
  third-party claims.
- No change to the existing `delete` meaning in this spec. Privacy-grade hard
  purge can be designed later as a separate command.
- No third-party app ingestion.

## Product Principles

### Canonical Store

The canonical user-context store remains relational:

- `user_context_claims` is the source of truth for durable user facts,
  preferences, constraints, goals, roles, project relationships, and activities.
- `user_context_candidates` is the review queue for automatic extraction.
- `user_context_summaries` is a compiled view, not truth.
- `memory_suppressions` and `memory_feedback` are policy and signal layers.

Markdown is only an export or view. It must never become a second writable
source of truth unless a future import flow explicitly maps edits back through
the same validation and source-ref rules as CLI edits.

### Profile Snapshot

Users should be able to produce a single Markdown snapshot of user context:

```bash
remem user profile export --format markdown --output profile.md
```

The snapshot should answer:

- What active user claims exist?
- What profile summary is currently active?
- Which source ids support the summary?
- Which claims are suppressed, deleted, expired, future, or restricted, when an
  audit flag requests them?
- When was the snapshot generated?

The default snapshot excludes inactive, suppressed, deleted, expired, future,
personal, sensitive, and restricted claims. Audit flags may include excluded
items, but the file must clearly label why each item is excluded from default
use.

The snapshot is a derived artifact. Editing it must not mutate remem state in
the first implementation.

### Usage Policy

Injected and recalled user context should affect answers only when it improves
the current task.

Rules for agents consuming user context:

- Use only source-attributed facts returned by remem. Do not infer profile data.
- Prefer invisible adaptation over explicit memory narration.
- Limit explicit memory mentions to 0-1 per response by default.
- Avoid phrases like "I remember you said" or "from previous conversations"
  unless the user is explicitly discussing memory, provenance, or correction.
- If user-context data is irrelevant, do not mention it.
- If user-context data conflicts with current live evidence, state the live
  evidence and treat memory as historical context.
- If no user context is returned, do not invent a generic user profile.

These rules should be visible in any user-context injection block and in MCP
tool instructions that return user context for agent consumption.

### Non-Retention Policy

Automatic extraction must not create active claims, candidates, or summary
inputs for content that is not durable user context.

Do not retain:

- temporary state, mood, fatigue, meals, weather, or one-off circumstances;
- world knowledge, project-independent facts, or general technical facts;
- third-party details unless the user explicitly frames them as relevant to
  their own durable context;
- guesses, jokes, sarcasm, role-play, fiction, or hypothetical identities;
- credentials, secrets, API keys, tokens, passwords, account numbers, identity
  documents, or payment data;
- illegal, harmful, or clearly false claims;
- assistant-authored claims about the user unless directly supported by a
  cited user-authored event;
- claims derived from files or external sources without explicit user approval.

If the extractor sees useful but non-retainable content, it should return
`no_candidates` or a review-gated candidate with an explicit block reason,
depending on the source and risk.

## User Stories

### Profile Snapshot

As a user, I can run a command and see the current user profile remem would use,
with source ids and exclusion reasons, so I can audit it without opening the
database.

Acceptance:

- The output is valid Markdown.
- The output has stable sections and stable ordering.
- The output clearly says it is a derived snapshot.
- The default output contains only active, default-eligible user context.
- `--include-sensitive` and `--include-suppressed` are explicit audit modes.
- Tests prove suppressed, deleted, expired, future, personal, sensitive, and
  restricted claims are excluded by default.

### Natural Usage

As a user, I want agents to benefit from remembered preferences without making
every answer feel like profile surveillance.

Acceptance:

- The context injection text includes usage rules for user context.
- Recall/MCP outputs include enough guidance for agents to use returned context
  naturally.
- Tests or snapshots prove the rules appear in rendered user-context output.

### Extraction Blocklist

As a user, I want automatic extraction to ignore transient, speculative, unsafe,
or non-user-profile content, so the profile does not become polluted.

Acceptance:

- The extraction prompt contains the non-retention blocklist.
- Parser/store behavior still fails closed on malformed output.
- Tests prove the prompt includes the blocklist.
- Tests prove blocklisted example candidates cannot auto-promote.

## Rollout

1. Profile Markdown snapshot.
2. Usage policy injection and recall instructions.
3. Automatic extraction blocklist and review block reasons.

Each phase should ship independently with tests. Runtime prompt changes require
focused extraction tests plus:

```bash
cargo fmt --check
cargo check
```

Run `cargo test` before merging any phase that changes extraction, context
rendering, MCP behavior, or schema.

## Open Questions

- Should profile snapshot live under `remem user profile export`, or should it
  extend `remem user summary show --format markdown`?
- Should snapshot import exist later, or should edits always go through
  `remem user claims edit` and `remem user summary edit`?
- Should privacy-grade hard purge be a separate `remem user purge --confirm`
  command with source-ref cleanup, or remain outside the first policy cycle?
