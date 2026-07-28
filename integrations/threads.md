# Threads Integration

SpecRail can provide optional advisory diagnostics alongside a threads-style
orchestration skill, but neither repository work nor threads depends on it. A
threads skill owns execution orchestration when work needs parallel lanes,
queue checks, review checks, or closure audits.

## Principle

- SpecRail is an optional advisory diagnostic and artifact layer.
- Threads is the optional execution layer.
- SpecRail checks may run before thread dispatch or before completion; their
  results do not block repository work.
- Missing thread support falls back to the normal single-agent repository flow.
- Stable machine-facing IDs stay unchanged across integrations and locales.

## When To Use Threads

Use an available threads skill when the task involves:

- a GitHub issue or pull request queue
- multiple independent implementation lanes
- read-only planner or reviewer lanes
- review thread, CI, or merge-readiness closure
- long-running work that needs a durable handoff

Do not use threads for a small single-file change, ordinary spec drafting, or
any task where all writable files overlap.

## Execution Order

1. Optionally collect SpecRail context or diagnostics when the user requests
   them or they are useful.
   - Load `AGENTS.md`, `workflow.yaml`, `states.yaml`, `labels.yaml`, relevant
     templates, and `skills/specrail-workflow/SKILL.md`.
   - Select the locale.
   - Record route, artifact, and verification findings as advisory context.
2. If the task needs queue or parallel orchestration, load the threads skill.
3. Run the threads capability and queue checks.
   - Confirm whether native subagents are available.
   - Fetch remote truth for GitHub queues.
   - Map issues to existing PRs before opening new work.
   - Build a lane map with disjoint writable files.
4. Execute lanes.
   - Planners and reviewers are read-only.
   - Workers own explicit writable paths.
   - The coordinator owns shared verification and final synthesis.
5. Optionally run SpecRail verification diagnostics.
   - Validate the pack.
   - Validate the spec packet when a spec changed.
   - Preserve human-facing locale rules.
   - Report findings without treating them as prerequisites for repository
     work.
6. Run threads closure audit when GitHub queue or PR state changed.
   - Re-check PR heads, CI, review threads, merge state, and issue closure.
   - Separate remote truth from local worktree state.

## Handoff Contract

Agents should record this block when both systems are active:

```yaml
specrail_threads_handoff:
  specrail:
    route:
    current_state:
    selected_locale:
    required_artifacts:
    human_gates:
    verification_commands:
  threads:
    mode:
    truth_level:
    queue_ledger:
    issue_to_pr_map:
    lanes:
    merge_policy:
    stop_conditions:
```

The block is a handoff artifact, not a schema-stable API. A future evaluator can
turn it into a validated artifact after repeated real use.

## Field Mapping

| SpecRail field | Threads field | Notes |
| --- | --- | --- |
| `route` | `intent_contract.goal` | The route defines the kind of workflow action. |
| `required_artifacts` | `queue_ledger.acceptance_evidence` | Threads records evidence for each queue item. |
| `human_gates` | `merge_policy`, `stop_conditions` | SpecRail findings are advisory; normal CI, code review, security decisions, and explicit human merge and release authorization still apply. |
| `verification_commands` | `verification_owner` | One owner runs shared checks for a tranche. |
| `selected_locale` | final report language | Human-facing reports follow SpecRail locale rules. |

## Fallback

If no threads skill or native subagent capability is available, the agent should
continue with the normal single-agent repository flow and say that no native
threads were launched. If the user explicitly requested threads, the agent may
provide a prompt pack and lane map instead of pretending parallel execution
happened.

## Non-Goals

- Do not vendor a local threads skill into SpecRail.
- Do not make threads required for adoption.
- Do not present SpecRail diagnostics as repository-level approval or blockers.
- Do not bypass normal CI, code review, security decisions, or explicit human
  merge and release authorization.
- Do not require GitHub for repositories that use SpecRail without GitHub.

## Minimal Agent Rule

For agents such as Codex:

```text
If the task is a queue, parallel-lane, review-thread, merge-readiness, or
closure-audit problem and a threads skill is available, use threads for
orchestration. SpecRail context and diagnostics are optional and advisory; they
do not decide whether repository work may proceed or merge.
```
