# User Context Layer Product Spec

Status: Current contract
Date: 2026-06-20

Tracking:
- Spec: #574
- Manual claims and CLI governance: #575
- Editable profile summaries: #576
- Suppression and feedback controls: #577
- On-demand user recall: #578
- Guarded automatic extraction: #579

## Problem

remem currently behaves like local, auditable engineering memory for coding
agents. It captures project/session evidence, distills durable coding facts,
and injects scoped repo context into Claude Code and Codex sessions.

The missing product layer is user context: stable facts about who the user is,
what they prefer, what they are trying to accomplish, which projects/tools they
work with, and what recent long-running activity matters across sessions.

This layer must not replace repo memory and must not become a hidden profile.
It should be an overlay that is local-first, source-backed, editable,
reviewable, suppressible, and retrieved only when relevant.

## Goals

- Let users explicitly record stable personal/work preferences, goals,
  constraints, and project relationships.
- Provide a compact, editable profile summary that can be shown, refreshed,
  corrected, and traced to sources.
- Support task-aware recall of user context without dumping all identity data
  into every SessionStart prompt.
- Preserve provenance for claims, summaries, suppressions, feedback, and later
  automatic extraction candidates.
- Make suppression distinct from deletion: "do not mention this again" should
  stop default use while preserving auditability unless the user explicitly
  confirms hard deletion.
- Keep sensitive/speculative identity claims out of active memory unless the
  user explicitly approves them.
- Reuse existing remem strengths: capture evidence, candidate review,
  owner-aware routing, current-state handling, usage feedback, and context
  injection audit.

## Non-Goals

- No hosted profile service.
- No third-party app ingestion in the first implementation.
- No Gmail/calendar/file-library memory source in the first implementation.
- No automatic sensitive identity inference or auto-promotion.
- No replacement of `memories`, `memory_candidates`, `workstreams`, or
  `session_summaries`.
- No hidden synthesis that users cannot inspect or correct.

## Product Model

The user context layer is a derived overlay:

```text
raw evidence / explicit user commands
  -> user-context candidates
  -> reviewed user-context claims
  -> optional activity timeline
  -> editable profile summary
  -> task-aware recall / compact context overlay
```

Phase 1 starts with explicit user commands only. Automatic extraction comes
later and must go through a review inbox unless the candidate is low-risk,
explicit, non-sensitive, and allowed by policy.

## Claim Types

User context claims use a vocabulary separate from coding memory types:

| Type | Meaning | Example |
|---|---|---|
| `identity` | Stable self-description explicitly provided by the user. | "The user maintains remem." |
| `role` | Work role or responsibility. | "The user is acting as product/architecture owner for remem." |
| `preference` | Response, workflow, tool, or communication preference. | "Prefer concise Chinese architecture analysis for remem." |
| `skill` | User skill or technical familiarity. | "Comfortable with Rust and local CLI workflows." |
| `goal` | Long-running objective. | "Make remem the best coding-agent memory system." |
| `project` | Relationship to a project, repo, or product. | "Works on remem." |
| `relationship` | Relationship to a person/org/project. | "Maintainer of repository X." |
| `constraint` | Explicit limitation or rule. | "Do not store private life details automatically." |
| `activity` | Recent user activity suitable for timeline recall. | "Shipped user-context spec PR." |

Coding memories keep the existing memory vocabulary: `decision`, `bugfix`,
`architecture`, `discovery`, `lesson`, `procedure`, `preference`, and
`session_activity`.

## Scope

User context supports the same ownership model used by the rest of remem:

| Scope | Use for |
|---|---|
| `user` | Stable preferences, goals, identity, and cross-project constraints. |
| `workspace` | Work habits or conventions that apply under one workspace root. |
| `repo` | User/project relationship or preference only relevant to one repo. |
| `session` | Short-lived scratch context that should not become durable profile. |

Default manual user claims use `owner_scope=user` and
`owner_key=user:default`. Commands must allow narrower scope only when the user
explicitly asks for it.

## Sensitivity

Every claim and candidate carries a sensitivity class:

| Sensitivity | Default behavior |
|---|---|
| `normal` | Can be active after explicit save or low-risk review approval. |
| `personal` | Requires explicit user wording or review before activation. |
| `sensitive` | Must stay pending review unless explicitly approved. |
| `restricted` | Never auto-promote; suppress or reject by default unless the user explicitly saves it. |

Examples that must not auto-promote:

- inferred location, employer, organization, private relationship, health,
  financial, political, religious, sexual, biometric, legal, or age-related
  details;
- speculative identity claims such as "the user might be...";
- claims derived from files or third-party sources without explicit approval.

## User Controls

### Manual Claims

Users can explicitly create and govern claims:

```bash
remem user remember "For remem, analyze from product and architecture first"
remem user remember --scope repo --owner-key /repo/path "For this repo, review specs before code"
remem user claims list
remem user claims why <id>
remem user claims edit <id>
remem user claims suppress <id>
remem user claims unsuppress <id>
remem user claims delete <id>
```

Default list/show commands exclude suppressed, rejected, deleted, expired, and
restricted claims unless an explicit admin flag includes them.
Manual claim creation defaults to `user:user:default`; narrower workspace, repo,
or session ownership must be explicit in the command and visible in `--json`
output.

### Profile Summary

The profile summary is a compiled view, not the source of truth:

```bash
remem user summary show
remem user summary refresh
remem user summary edit
remem user summary sources
```

Users can edit the summary, but the system must keep source ids so later
refreshes can explain what changed. Refresh failures must preserve the last good
summary and report an actionable error.

### Suppression and Feedback

Users can stop unwanted context without deleting evidence:

```bash
remem memory suppress memory:123 --reason "not relevant anymore"
remem memory unsuppress memory:123 --reason "needed again"
remem memory feedback memory:123 --value not_relevant
remem memory suppressions list
```

Suppressed items are excluded from default context and recall. Explicit admin
commands may still inspect them for audit.

### On-Demand Recall

SessionStart should stay compact. Agents can request relevant user context when
it helps the current task:

```bash
remem user recall "analyze the remem user memory design"
```

MCP should expose the same behavior with structured inputs such as query,
project/cwd, host, current files, task intent, owner filters, and sensitivity
flags.

Recall output must include source ids and reason codes. Empty recall returns a
clear empty result; it must not invent a generic profile.

## Default Context Behavior

SessionStart may include only a small user/profile overlay once claims and
summary support are implemented:

- stable user preferences relevant to the current host/repo;
- active project relationship when clearly relevant;
- current workstream or goal when scoped to the current repo;
- no personal, sensitive, restricted, suppressed, rejected, expired, or unrelated
  claims unless the claim was explicitly approved for startup context.

Long-tail user context is retrieved through on-demand recall, not injected by
default.

## Phased Rollout

### Phase 1: Manual Claims (#575)

Deliver explicit user-context claim storage and CLI governance. This proves the
data model, status transitions, source refs, and user controls before any
automatic inference.

### Phase 2: Editable Summary (#576)

Compile active claims and relevant project memory into a compact profile
summary with source traceability. Keep summaries derived and reversible.

### Phase 3: Suppression and Feedback (#577)

Add first-class suppress/unsuppress and relevant/not_relevant feedback controls
for claims, memories, topics, and injected context items.

### Phase 4: On-Demand Recall (#578)

Add CLI/MCP recall that composes user claims, profile summary, repo memory,
current-state answers, workstreams, and recent sessions for one task.

### Phase 5: Guarded Automatic Extraction (#579)

Only after manual controls are stable, add user-context extraction candidates
with review gates, sensitivity classification, explicit block reasons, and
fail-closed parsing.

## Acceptance

The user context layer is product-ready when:

- users can see what is remembered about them and why;
- users can edit, suppress, reject, or delete user-context claims;
- sensitive/speculative claims cannot silently become active;
- profile summaries never hide their source data;
- context injection remains compact and owner-aware;
- on-demand recall returns source-attributed context or a clear empty result;
- tests prove that suppressed/sensitive/rejected/expired data is excluded from
  default recall and injection.

## References

- Existing remem architecture: `docs/ARCHITECTURE.md`
- Existing owner routing background: `docs/spec-memory-ownership-routing.md`
- Existing governance background: `docs/spec-memory-governance-v2.md`
- Existing context compiler background: `docs/spec-context-compiler.md`
- Public product reference for memory summaries, sources, feedback, and
  relevance-triggered recall: <https://help.openai.com/en/articles/8590148-memory-faq>
