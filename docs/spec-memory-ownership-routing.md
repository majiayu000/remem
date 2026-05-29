# Spec: Memory Ownership and Routing

**Status**: Draft
**Date**: 2026-05-28
**Related**:
- `docs/spec-context-compiler.md`
- `docs/memory-lifecycle.md`
- `docs/temporal-facts.md`
- `docs/workstream-design.md`

## 1. Problem

remem currently uses the active working directory as the primary durable
ownership key:

```text
cwd -> project_from_cwd(cwd) -> memories.project
```

This is technically safe for project filtering, but it is not semantically
correct. A user can be inside one repository while asking about another tool,
host, API, or workflow. Those extracted memories become project-scoped to the
wrong repository and later appear in SessionStart context.

A concrete audit from `/Users/lifcc/Desktop/code/AI/tool/stash` showed:

- Stash-specific UI, DnD, PR, and dev-server memories were correctly scoped.
- Codex CLI sandbox/approval memories were stored as Stash project memories.
- Grok, Warp, Hermes, and generic API-client workstreams were also present
  under the Stash project.
- Several user preferences were duplicate or domain-wide, not Stash-specific.

The retrieval code did not leak across projects. The write-time ownership model
was too coarse.

## 2. Why This Matters

Bad ownership is worse than weak ranking:

- SessionStart injects unrelated memory before the current task is known.
- Project context becomes noisy and stale.
- WorkStreams accumulate unrelated active tasks.
- Preferences that should be global or domain-specific masquerade as repo
  facts.
- Later retrieval improvements cannot reliably fix incorrectly owned data.

The product goal remains memory quality, not minimal cost. The correct fix is a
stronger write path and lifecycle, not removing automatic capture.

## 3. External Patterns To Borrow

The external systems point to the same shape:

- LangGraph long-term memory stores JSON documents under custom namespaces and
  keys, commonly including user/org/context labels. Cross-namespace retrieval
  is done with filters.
- Mem0 search emphasizes scoped datasets and metadata filters such as user,
  agent, run/session, date, and category. Its update operation also updates
  metadata so filters stay accurate.
- Letta separates always-visible core memory blocks from large archival memory
  that is searched on demand.
- Zep builds user-level temporal knowledge graphs from session messages and
  returns an engineered context string with temporal facts and entities.
- Mem0's temporal reasoning work treats time-sensitive current facts
  differently from historical facts by extracting timing and state metadata.

These patterns imply:

```text
write-time routing + explicit namespace + metadata filters + lifecycle
```

They do not require adopting a vector database or graph database as the first
step.

## 4. Goals

- Stop using `cwd` as the only durable ownership signal.
- Preserve the observed source directory for provenance.
- Add explicit ownership metadata that can represent user, repository, tool,
  workstream, and session scopes.
- Route memories before they become active project context.
- Make low-confidence or cross-domain memories reviewable instead of silently
  promoted.
- Keep active SessionStart context small and high-signal.
- Add lifecycle semantics for temporary git, PR, service, and workstream facts.
- Support cleanup/migration of existing polluted project memory.

## 5. Non-Goals

- No vector database dependency.
- No graph database rewrite.
- No deletion of historical records as the normal cleanup mechanism.
- No broad redesign of MCP tool contracts in the first slice.
- No attempt to perfectly classify every memory automatically. Ambiguous cases
  should be deferred or queued for review.

## 6. Ownership Model

### 6.1 Current Fields

The current durable fields are:

```text
project
scope        # project | global
memory_type  # decision | discovery | preference | bugfix | ...
branch
topic_key
status       # active | stale | ...
```

They are insufficient because `project` means both "where the conversation
happened" and "what this memory is about."

### 6.2 New Fields

During Slice 1, add nullable ownership fields to `memories`,
`memory_candidates`, `workstreams`, and `session_summaries`. They stay nullable
while existing rows are backfilled and while staged candidates are being routed:

```sql
source_project TEXT,
target_project TEXT,
owner_scope TEXT,
owner_key TEXT,
topic_domain TEXT,
routing_confidence REAL,
routing_reason TEXT,
expires_at_epoch INTEGER,
valid_from_epoch INTEGER,
valid_to_epoch INTEGER
```

Field meanings:

| Field | Meaning |
|---|---|
| `source_project` | The `project_from_cwd(cwd)` value at capture time. |
| `target_project` | The repository/project this memory is actually about, when applicable. |
| `owner_scope` | One of `user`, `workspace`, `repo`, `tool`, `domain`, `workstream`, `session`. |
| `owner_key` | Stable key inside the scope, such as a workspace root, repo path, `codex-cli`, or `grok-api`. |
| `topic_domain` | Coarse domain label used for routing and cleanup, such as `stash-ui` or `codex-sandbox`. |
| `routing_confidence` | Classifier confidence in `0.0..=1.0`. |
| `routing_reason` | Short explanation for audit and review. |
| `expires_at_epoch` | Optional TTL for ephemeral facts. |
| `valid_from_epoch` / `valid_to_epoch` | Validity window for temporal/current facts. |

Post-backfill, new writes must populate `source_project`, `owner_scope`, and
`owner_key` before activation. Enforce that invariant in the write path first;
a later table rebuild or check constraint can make those fields non-null once
all supported SQLite migrations have completed.

Backfill rule:

```text
For memories, memory_candidates, and workstreams:
  project_text =
    normalized schema: projects.project_path via <table>.project_id
    legacy tables: project

  workspace_text =
    normalized schema: workspaces.root_path via projects.workspace_id
    legacy tables: NULL

  source_project = project_text

  if scope == 'global':
    target_project = NULL
    owner_scope = 'user'
    owner_key = 'user:default'

  if scope == 'workspace':
    target_project = NULL
    owner_scope = 'workspace'
    owner_key = workspace_text, falling back to project_text

  if scope == 'project':
    target_project = project_text
    owner_scope = 'repo'
    owner_key = project_text

For session_summaries:
  source_project =
    normalized schema: projects.project_path via session_summaries.project_id
    legacy tables: project
  owner_scope/owner_key remain NULL until the summary is rerouted, unless a
  deterministic backfill rule can prove repo/workspace ownership.
```

Do not treat `source_project = current project` as enough to inject a legacy
session summary into SessionStart context. Session history needs explicit
`owner_scope` / `owner_key` routing or a high-confidence deterministic
backfill.

Keep `project` during migration for compatibility. New query paths should move
toward `target_project` / `owner_scope` / `owner_key` filters, then `project`
can become a compatibility alias.

### 6.3 Ownership Scopes

| Scope | Use for | Example owner_key |
|---|---|---|
| `user` | Stable user preferences and communication preferences. | `user:default` |
| `workspace` | Workspace-level rules or facts intended to apply across repos under one workspace root. | `/Users/lifcc/Desktop/code/AI` |
| `repo` | Facts about one code repository or product. | `/Users/lifcc/Desktop/code/AI/tool/stash` |
| `tool` | Cross-repo facts about a tool/runtime. | `codex-cli`, `claude-code`, `gh-cli` |
| `domain` | Cross-repo technical domain or external API. | `grok-api`, `macos-tcc`, `npm-publish` |
| `workstream` | Task-state memory tied to one workstream. | `workstream:758` |
| `session` | Ephemeral session scratch that should not persist into project context. | `session:<id>` |

## 7. Routing Classifier

### 7.1 Placement

The classifier runs before durable activation:

```text
raw event/session summary
  -> memory candidate
  -> route candidate ownership
  -> lifecycle decision
  -> promote/update/stale/defer
```

This belongs in the promotion path, not in SessionStart retrieval. Retrieval
should not have to guess that a memory was misfiled.

### 7.2 Routing Inputs

The classifier should see:

- cwd/source project
- session request
- session completed/next steps
- memory title/content
- memory type
- files modified/read
- git branch and remote when available
- linked workstream if any
- explicit user text such as "this repo", "Codex", "Grok", "Warp", or "Stash"

### 7.3 Deterministic Rules

Apply high-confidence rules before LLM classification:

| Signal | Route |
|---|---|
| Files under current repo are modified/read and title/content names local components | `repo:<current project>` |
| Content is about current repo dev server, tests, PRs, branches, local scripts | `repo:<current project>` |
| Content is about Codex CLI approvals, sandbox, MCP config, or hooks | `tool:codex-cli` unless a repo file path proves repo ownership |
| Content is about Claude Code hooks/runtime | `tool:claude-code` or current repo if it changes remem integration code |
| Content is about GitHub CLI usage, issue/PR workflow, or Actions patterns across repos | `tool:gh-cli` or `domain:github-workflow` |
| Content is about Grok/xAI API usage or wrappers | `domain:grok-api` |
| Content is about macOS TCC, app bundles, Sparkle updates, or system routing | `domain:macos` |
| Communication preference without repo nouns | `user:user:default` |
| Temporary observations with no durable future value | `session:<id>` or `noop` |

If deterministic rules disagree, defer to review unless there is direct file
evidence for repo ownership.

### 7.4 LLM Routing Output

When rules are inconclusive, ask the existing memory extractor/summary model to
return a routing block:

```xml
<memory_route>
  <owner_scope>user|workspace|repo|tool|domain|workstream|session</owner_scope>
  <owner_key>...</owner_key>
  <target_project>...</target_project>
  <topic_domain>...</topic_domain>
  <confidence>0.0-1.0</confidence>
  <durability>permanent|current|ephemeral|noop|defer</durability>
  <reason>...</reason>
</memory_route>
```

Promotion thresholds:

| Confidence | Action |
|---|---|
| `>= 0.85` | Auto-promote with route metadata. |
| `0.60..0.85` | Promote only if deterministic rules agree; otherwise pending review. |
| `< 0.60` | Defer/pending review. |

## 8. Lifecycle Rules

Ownership and lifecycle should be decided together.

### 8.1 Durability Classes

| Durability | Default behavior |
|---|---|
| `permanent` | Long-lived product, architecture, bugfix, or user preference. |
| `current` | Current operational state; visible until superseded or expired. |
| `ephemeral` | Session-level note; searchable in raw/session history but not SessionStart. |
| `noop` | Do not write durable memory. |
| `defer` | Keep candidate for review/retry. |

### 8.2 TTL Defaults

| Fact type | Default TTL |
|---|---|
| Service currently running, port occupied, local URL healthy | 24 hours |
| PR mergeability, CI state, review status | 24 hours |
| Git branch divergence snapshot | 7 days |
| Workstream next action | 14 days without activity before pause |
| Product/architecture decision | No TTL |
| Verified bugfix with files/tests | No TTL |
| User communication preference | No TTL, but must merge/update duplicates |

Expired rows should become `stale`, not deleted.

### 8.3 State Keys

Use `state_key`-like topic keys for evolving facts:

```text
repo:<path>:git-divergence
repo:<path>:dev-server
repo:<path>:pr:<number>:mergeability
tool:codex-cli:sandbox-model
user:default:communication-style
```

When a new active fact with the same state key arrives, use the existing
`update` lifecycle operation: insert the replacement and mark old rows stale.

## 9. Context Compiler Changes

SessionStart should use layered retrieval, not a flat project dump.

### 9.1 Layers

```text
User Core
  owner_scope=user, owner_key=user:default, stable preferences only

Workspace Core
  owner_scope=workspace, owner_key=current workspace, stable workspace facts only

Repo Core
  owner_scope=repo, owner_key=current project, permanent architecture/bugfix/decision

Active WorkStreams
  owner_scope=repo/workstream for current project, status active by default

Task-Aware Retrieval
  optional query-aware search when hook input includes the latest user prompt

Session History
  recent summaries filtered by owner_scope/owner_key for current repo or
  workspace, capped and deduped

Archival Index
  compact pointers, not full details
```

### 9.2 Default SessionStart Policy

For startup with no user task text:

- include stable user preferences
- include stable workspace-scoped memories for the current workspace
- include repo core decisions/bugfixes
- include active workstreams for current repo
- include recent sessions only if high-signal, not stale, and routed to the
  current repo/workspace owner
- do not include tool/domain memory unless it is linked to the repo or active
  workstream
- do not include paused workstreams by default; include them only in
  task-aware retrieval when the prompt implies resumption and the paused row
  matches the current owner with a recent `updated_at_epoch`

For prompt-submit or task-aware context:

- retrieve relevant tool/domain memory only when the current user prompt asks
  about that domain
- use explicit filters, for example:

```text
(owner_scope=repo AND owner_key=current_project)
OR (owner_scope=user AND owner_key=user:default)
OR (owner_scope=workspace AND owner_key=current_workspace)
OR (owner_scope=tool AND owner_key IN inferred_tools)
OR (owner_scope=domain AND owner_key IN inferred_domains)
```

### 9.3 Rendering Changes

Footer should expose routing counts:

```text
31 context memories loaded. repo=18 user=5 tool=0 domain=0 workstreams=3 sessions=5 ...
```

Debug trace should show:

```text
id=76728 source_project=stash owner_scope=tool owner_key=codex-cli excluded reason=tool_not_relevant_to_startup
```

## 10. Cleanup and Review Tools

### 10.1 Audit Command

Add:

```text
remem audit-scope --project <project> [--limit N] [--json]
```

Output categories:

- likely correct repo memory, with `object_ref` such as `memory:76724`
- likely cross-tool/domain pollution, with `object_ref`
- duplicate preferences, with `object_ref`
- duplicate workstreams, with `object_ref` such as `workstream:18`
- stale temporal facts, with `object_ref`
- low-confidence routing, with `object_ref`

Audit and write commands must use object-qualified references, not raw integer
IDs. Valid prefixes are `memory`, `candidate`, `workstream`, and
`session-summary`.

### 10.2 Migration Command

Add:

```text
remem reroute --refs memory:76724,memory:76728 --owner-scope tool --owner-key codex-cli --clear-target-project
remem archive --refs memory:76731,workstream:18
remem merge-preferences --project <project> --dry-run
```

`--clear-target-project` stores SQL `NULL`. Empty-string target projects should
be rejected or normalized to `NULL`; they must not become a second "no target"
representation.

All commands should be dry-run by default and require `--confirm` for writes.

### 10.3 Stash Cleanup Seed Case

The first manual cleanup fixture should encode the observed Stash pollution:

- keep Stash UI/DnD/product/dev-server memories under Stash repo
- reroute Codex sandbox/approval memories to `tool:codex-cli`
- archive or pause unrelated Grok/Warp/Hermes workstreams under Stash
- merge duplicate UI critique preferences

This should become a regression fixture for the routing classifier.

## 11. Data Migration Plan

### Slice 1: Schema and Backfill

- Add nullable ownership fields to `memories`, `memory_candidates`,
  `workstreams`, and `session_summaries`.
- Backfill `memories`, `memory_candidates`, and `workstreams` from existing
  `project` and `scope`; backfill legacy `session_summaries` with
  `source_project` only unless ownership can be proven.
- Add indexes:

```sql
CREATE INDEX idx_memories_owner_status
  ON memories(owner_scope, owner_key, status, updated_at_epoch DESC);

CREATE INDEX idx_memories_source_project
  ON memories(source_project, updated_at_epoch DESC);

CREATE INDEX idx_memories_target_project_status
  ON memories(target_project, status, updated_at_epoch DESC);

CREATE INDEX idx_workstreams_owner_status
  ON workstreams(owner_scope, owner_key, status, updated_at_epoch DESC);

CREATE INDEX idx_session_summaries_owner_created
  ON session_summaries(owner_scope, owner_key, created_at_epoch DESC);

CREATE INDEX idx_session_summaries_source_project
  ON session_summaries(source_project, created_at_epoch DESC);
```

### Slice 2: Write Path Routing

- Create `memory::routing`.
- Route candidates before promotion.
- Store route metadata and confidence.
- Pending-review low-confidence routes.
- Add deterministic tests for Stash/Codex/Grok/Warp examples.

### Slice 3: Context Compiler Filters

- Update context loading to use owner filters.
- Add tool/domain exclusion at startup.
- Add debug trace and footer counts.
- Keep compatibility fallback to `project` until migration is complete.

### Slice 4: Lifecycle and Cleanup

- Add TTL handling for ephemeral/current facts.
- Add stale transition job.
- Add audit/reroute/merge commands.
- Clean the Stash dataset as a real fixture-backed validation.

## 12. Testing Plan

### Unit Tests

- `routing::classifies_repo_file_changes_as_repo`
- `routing::classifies_codex_sandbox_as_tool`
- `routing::classifies_grok_api_as_domain`
- `routing::classifies_plain_preference_as_user`
- `routing::defers_conflicting_signals`
- `context::startup_excludes_unrelated_tool_memory`
- `context::debug_trace_reports_owner_exclusion`
- `lifecycle::ttl_expiry_marks_current_fact_stale`
- `workstream::duplicate_title_updates_existing_owner`

### Integration Tests

- Seed a Stash-like project with mixed Stash/Codex/Grok/Warp memories.
- Run `remem context --cwd <stash> --host codex-cli --debug`.
- Assert Stash repo memories appear.
- Assert Codex/Grok/Warp memories do not appear at startup.
- Run search with explicit tool/domain filters and assert rerouted memories are
  still retrievable.

### Local Verification

Before completion:

```bash
cargo fmt --check
cargo check
cargo test context:: memory:: workstream:: --lib
```

Before submission:

```bash
cargo test
```

## 13. Open Questions

- Should `project` remain a compatibility alias forever, or eventually become
  `source_project` only?
- Should `owner_key` for repos use absolute paths, canonical git remotes, or
  both?
- Should global user preferences be stored as structured profile fields instead
  of regular memories?
- Should domain/tool memory have separate MCP search commands, or only filters?
- Should routing review use existing `pending_observations` or a new
  `memory_routing_reviews` table?

## 14. Proposed GitHub Issues

1. Add ownership namespace schema and compatibility backfill.
2. Add write-time memory routing classifier and review thresholds.
3. Update Context compiler to use layered owner-aware retrieval.
4. Add temporal TTL lifecycle for current/ephemeral memories and workstreams.
5. Add audit/reroute/merge cleanup commands and clean the Stash pollution case.

## 15. References

- LangGraph long-term memory: namespace/key JSON store with filters.
  https://docs.langchain.com/oss/python/langchain/long-term-memory
- Mem0 search: scoped datasets, metadata filters, reranking, timestamps.
  https://mem0.mintlify.app/core-concepts/memory-operations/search
- Mem0 update: update memory content and metadata together.
  https://docs.mem0.ai/core-concepts/memory-operations/update
- Letta memory blocks: always-visible core memory blocks.
  https://docs.letta.com/guides/core-concepts/memory/memory-blocks
- Letta archival memory: large searchable out-of-context memory.
  https://docs.letta.com/guides/ade/archival-memory
- Zep concepts: engineered memory context from temporal facts and entities.
  https://help.getzep.com/v2/concepts
- Mem0 temporal reasoning: current/historical memory metadata and state keys.
  https://mem0.ai/blog/the-token-efficient-memory-algorithm-now-has-temporal-reasoning
