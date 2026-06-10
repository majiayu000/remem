# Spec: Memory Governance V2

Status: Draft
Date: 2026-05-31

Related:
- `docs/spec-memory-ownership-routing.md`
- `docs/memory-lifecycle.md`
- `docs/spec-autodream.md`
- `docs/ref/memory-retrieval-research-2026-05-24.md`
- `docs/audit-2026-05-29.md`
- Tracking issue: <https://github.com/majiayu000/remem/issues/274>

## Problem

remem already has the pieces of a strong memory system: raw capture,
observations, memory candidates, typed curated memories, lifecycle helpers,
governance actions, dream consolidation, project-scoped SessionStart context,
and search tools.

The current product risk is that those pieces do not form one enforced write
and read policy:

- `cwd` is still treated as the durable project owner, so one repository can
  absorb memories about tools, domains, other repos, or the user's general
  workflow.
- summary promotion can write curated memories directly, bypassing candidate
  review, routing, conflict checks, and lifecycle decisions.
- candidate promotion can add/upsert memories, but it does not first compare
  against current active memories to choose `update`, `invalidate`, `noop`, or
  `defer`.
- `topic_key` upsert is syntactic, not semantic. Content-hash keys make many
  corrected facts append instead of update.
- SessionStart context filters by active status and project, but an incorrect
  active memory can still be injected.
- dream/governance exists, but it is not yet an automatic, observable quality
  loop.

The desired behavior is not "store less memory." The desired behavior is to
store the right evidence, promote only suitable durable facts, update or stale
obsolete facts, and inject only memories that are safe and relevant for the
current owner and task.

## Research Requirements

Recent memory-system papers point in the same direction:

1. Raw episodes must remain first-class evidence. Continuous LLM consolidation
   can make useful memories faulty; consolidation should be delayed, gated, and
   recoverable from raw trajectories.
   Source: Useful Memories Become Faulty When Continuously Updated by LLMs,
   arXiv:2605.12978.

2. Stale memory is not only an old-record problem. Later observations can
   implicitly invalidate earlier beliefs, and agents must resist false-premise
   queries that assume stale state is still true.
   Source: STALE, arXiv:2605.06527.

3. Conflict handling is query-conditioned. A memory can be temporally valid,
   factually correct, but not applicable to the current query or owner.
   Source: MemConflict, arXiv:2605.20926.

4. Production memory systems extract, consolidate, and retrieve salient
   information with metadata and scope, rather than treating the memory bank as
   a flat append log.
   Source: Mem0, arXiv:2504.19413.

5. Agentic memory systems benefit from structured notes, dynamic links, and
   updating the representation of existing memories when new evidence arrives.
   Source: A-MEM, arXiv:2502.12110.

6. Long-term AI memory needs explicit modeling of object, form, and time. For
   remem, that maps to owner, memory type, validity/currentness, and lifecycle.
   Source: From Human Memory to AI Memory, arXiv:2504.15965.

7. Agent memory should be evaluated as a write-manage-read loop, including
   filtering, contradiction handling, latency, and privacy governance.
   Source: Memory for Autonomous LLM Agents, arXiv:2603.07670.

8. Benchmarks should cover knowledge update, temporal reasoning, abstention,
   and multi-session reasoning rather than only self-retrieval.
   Sources: LoCoMo, arXiv:2402.17753; LongMemEval, arXiv:2410.10813.
   LoCoMo is informational-only for remem and must not be used as a CI gate.

These sources support a local-first, auditable architecture:

```text
raw episode
  -> observation
  -> candidate
  -> ownership route
  -> lifecycle operation
  -> curated memory / stale old memory / noop / review
  -> owner-aware context compiler
  -> quality audit and consolidation loop
```

## Goals

- Use repository root and explicit ownership as durable routing signals.
- Keep source provenance separate from target ownership.
- Make all curated writes pass through one candidate + lifecycle decision path.
- Replace append-by-default with `add | update | invalidate | noop | defer`.
- Preserve raw evidence and stale rows for audit; do not rely on destructive
  cleanup.
- Prevent unsuitable memories from entering SessionStart context.
- Add realistic sandbox tests for project ownership, memory update, conflict,
  branch filtering, false premise, and context injection.
- Keep the crate a single Rust binary with SQLite and existing local-first
  constraints.

## Non-Goals

- No vector database requirement.
- No graph database rewrite.
- No hard deletion as the normal correction mechanism.
- No live mutation of the user's real memory database as part of tests.
- No release until spec, issues, PRs, review, full tests, and renewed audit are
  complete.

## Design Principles

### Evidence First

Raw archive rows and captured events are the source of truth. Curated memories
are derived claims. If a curated memory is wrong, the system must be able to
explain which evidence created it and which newer evidence superseded it.

### Ownership Before Promotion

`source_project` answers "where did this conversation happen?" `owner_scope`
and `owner_key` answer "where should this memory be used?" A memory captured
while the user is inside repo A can still be owned by `tool:codex-cli`,
`domain:grok-api`, `user:user:default`, or repo B.

### Lifecycle Before Insertion

Promotion is a decision, not an insert helper. The promotion path must load
nearby active memories and choose one operation:

| Operation | Meaning | Effect |
|---|---|---|
| `add` | New durable fact. | Insert active memory. |
| `update` | New fact replaces older active facts. | Insert replacement and stale superseded ids. |
| `invalidate` | Existing fact is known obsolete or wrong. | Mark listed ids stale. |
| `noop` | Evidence is already represented or not durable. | Write no curated memory; record reason. |
| `defer` | Evidence, ownership, or conflict is unsafe to decide. | Leave curated memory unchanged; review/retry. |

### Context Is a Policy Surface

Searchable is not the same as injectable. Context compilation should use an
explicit policy field or derived policy:

| Context class | Default behavior |
|---|---|
| `startup_core` | Can enter SessionStart for matching owner. |
| `task_relevant` | Searchable/injectable only when prompt or tool context matches. |
| `search_only` | Never injected automatically; returned only by explicit search. |
| `never_inject` | Kept for audit/history, not context. |

## Implementation Roadmap

### Issue 1: Project Identity and Ownership Schema

GitHub: <https://github.com/majiayu000/remem/issues/220>

Base project identity on canonical git repo root when available, falling back to
canonical `cwd`. Preserve `source_cwd` or equivalent provenance separately.

Add nullable ownership and validity fields to the relevant durable tables:

- `source_project`
- `target_project`
- `owner_scope`
- `owner_key`
- `topic_domain`
- `routing_confidence`
- `routing_reason`
- `context_class`
- `expires_at_epoch`
- `valid_from_epoch`
- `valid_to_epoch`

Backfill compatibility rows conservatively. Legacy repo-scoped rows may default
to `owner_scope='repo'` and `owner_key=project` only when the existing scope is
project-local. Legacy session summaries should keep `source_project` only until
routed.

Required tests:

- cwd under nested repo maps to the repo root owner.
- non-git directories still map to canonical cwd.
- migration backfills legacy rows without dropping old `project` behavior.
- local Markdown backup paths include a stable project key/hash to avoid
  collisions.

### Issue 2: Candidate Routing and Lifecycle Decisions

GitHub: <https://github.com/majiayu000/remem/issues/221>

Route each candidate before promotion. The route can be deterministic first,
then LLM-assisted only when necessary.

Candidate outputs must include:

- `op`
- `owner_scope`
- `owner_key`
- `target_project`
- `topic_domain`
- `context_class`
- `routing_confidence`
- `routing_reason`
- `supersedes`
- `conflicts`
- `evidence_event_ids`

Before applying the operation, load active memories for the same owner,
topic/entity, memory type, branch, and candidate text terms. Use those rows to
choose `add`, `update`, `invalidate`, `noop`, or `defer`.

Required tests:

- duplicate evidence becomes `noop`, not a second active memory.
- newer conflicting fact marks old memory stale.
- low-confidence routing stays pending review.
- Codex sandbox facts route to `tool:codex-cli`, not the current repo.
- user communication preferences route to `user:user:default`.
- auto-promote never succeeds without evidence ids and a route.

### Issue 3: Remove Direct Summary Promotion

GitHub: <https://github.com/majiayu000/remem/issues/272>

Stop summary finalization from writing curated memories directly. Session
summaries can still be stored as summaries and raw evidence, but any durable
memory derived from a summary must become a candidate and pass through routing
and lifecycle.

Required tests:

- summary with decisions creates candidates, not active memories.
- malformed candidate output fails closed.
- summary-derived candidates retain source session and evidence range.
- existing summary context rendering remains compatible.

### Issue 4: Owner-Aware Context Compiler

GitHub: <https://github.com/majiayu000/remem/issues/222>

SessionStart should load layered context:

1. stable user preferences for `user:user:default`
2. workspace facts for the current workspace
3. repo core for the current repo owner
4. active workstreams for the current repo/workstream owner
5. recent routed sessions only when high signal and owner matched
6. task-aware tool/domain memory only when prompt metadata implies relevance

Default startup must not inject unrelated tool/domain/session memories.

Required tests:

- active wrong-owner memory is excluded from startup context.
- branch-specific repo memory from another branch is excluded by default.
- stale/rejected/deleted memories stay excluded.
- unknown custom memory types do not enter MemoryIndex by default.
- debug output shows include/exclude reason counts.

### Issue 5: Dream and Governance Automation

GitHub: <https://github.com/majiayu000/remem/issues/223> and
<https://github.com/majiayu000/remem/issues/224>

Make dream/governance an observable quality loop after routing and lifecycle are
in place. Automation must be conservative:

- dry-run first
- cooldown per owner
- audit events retained in a durable governance log
- no hard delete by default
- prompt input includes updated time, owner, branch, evidence ids, and current
  status
- LLM merge cannot change owner/type/scope without validation

Required tests:

- dream dry-run lists clusters without writes.
- dream merge stales superseded rows and preserves provenance.
- cooldown prevents repeated owner runs.
- governance audit survives normal event cleanup.
- include-stale historical search can find stale rows when requested.

### Issue 6: Realistic Memory Quality Eval

GitHub: <https://github.com/majiayu000/remem/issues/273>

Add a sandboxed quality suite that does not touch the user's real database.

Scenarios:

- nested repo cwd split: `/repo`, `/repo/src`, `/repo/crates/x`
- cross-tool pollution: repo session mentions Codex approvals, Grok API, Warp
  config, and local repo files
- update conflict: old preference/fact becomes invalid after newer evidence
- false premise: query assumes stale state and should not retrieve it as current
- branch mismatch: feature-branch memory should not inject on main
- summary promotion: summary-derived facts must pass candidate lifecycle
- context injection: startup context includes only safe owner-matched core

Metrics:

- evidence recall at k
- active-current precision
- stale exclusion rate
- owner routing accuracy
- noop/update/invalidate/defer counts
- context injection precision

Required verification:

```bash
cargo fmt --check
cargo check
cargo test
```

## PR Sequence

Each issue should be implemented as a separate PR against `main`:

1. Spec PR: add this spec and create/update GitHub issues.
2. Issue 1 PR: schema and project identity.
3. Issue 2 PR: routing + lifecycle operation path.
4. Issue 3 PR: summary promotion reroute.
5. Issue 4 PR: owner-aware context compiler.
6. Issue 5 PR: dream/governance automation.
7. Issue 6 PR: realistic eval suite and final audit fixes.

Every PR must include:

- linked issue
- focused diff
- tests for the changed behavior
- `cargo fmt --check`
- `cargo check`
- relevant focused `cargo test ...`
- code-review pass before merge

The final PR must also include a renewed audit report showing which failure
modes are fixed, which risks remain, and which commands produced the evidence.

## Stop Rules

- Stop before mutating the user's real remem database.
- Stop before force-pushing or resetting local `main`.
- Stop if an issue requires a product decision not covered by this spec.
- Stop after three failed fixes on the same symptom and revisit the hypothesis.
- Stop if GitHub auth, CI, or branch protection prevents issue/PR/merge work.

## References

- Useful Memories Become Faulty When Continuously Updated by LLMs:
  https://arxiv.org/abs/2605.12978
- STALE: Can LLM Agents Know When Their Memories Are No Longer Valid?:
  https://arxiv.org/abs/2605.06527
- MemConflict: Evaluating Long-Term Memory Systems Under Memory Conflicts:
  https://arxiv.org/abs/2605.20926
- Mem0: Building Production-Ready AI Agents with Scalable Long-Term Memory:
  https://arxiv.org/abs/2504.19413
- A-MEM: Agentic Memory for LLM Agents:
  https://arxiv.org/abs/2502.12110
- From Human Memory to AI Memory:
  https://arxiv.org/abs/2504.15965
- Memory for Autonomous LLM Agents:
  https://arxiv.org/abs/2603.07670
- A Survey on the Memory Mechanism of Large Language Model based Agents:
  https://arxiv.org/abs/2404.13501
- LoCoMo:
  https://arxiv.org/abs/2402.17753
  Informational-only for remem; not a CI or release gate.
- LongMemEval:
  https://arxiv.org/abs/2410.10813
