# Preference Rule Compilation Technical Spec

Status: Current contract
Date: 2026-07-02

Tracking:
- Spec/tracking issue: #671
- Related umbrella: #383

## Existing Implementation Facts

- Preferences are a first-class memory type (`src/memory/types.rs`), rendered
  as a dedicated section in the SessionStart context block
  (`src/context/render.rs`).
- Lesson/preference metadata carries `reinforcement_count`
  (`src/memory_candidate/apply.rs`).
- Hooks are dispatched through `src/cli/dispatch.rs`; Claude Code fires
  SessionStart, UserPromptSubmit, PostToolUse, and Stop; Codex fires
  SessionStart/Stop with PostToolUse(Bash) opt-in.
- The background worker (`src/worker.rs`) already runs extraction and
  consolidation off the interactive path.
- `memory_suppressions` (v051) and soft supersession
  (`src/memory/lifecycle.rs`) define when a memory stops being authoritative.

## Design Rules

- SQLite remains canonical; the compiled rules artifact is derived output.
- Hooks read the artifact; only the worker writes it.
- No LLM, network, or DB write in hook-side evaluation.
- Every rule is traceable to exactly one source memory id.
- Fail open: unreadable artifact means no enforcement plus one error-level log
  per session, never a crashed or blocked hook.

## Proposed Design

### Rule artifact

Derived file per project at `~/.remem/compiled_rules/<project-hash>.json`:

```json
{
  "version": 1,
  "compiled_at_epoch": 0,
  "rules": [
    {
      "rule_id": "pref-<memory_id>-1",
      "source_memory_id": 123,
      "reinforcement_count": 3,
      "action": "warn",
      "predicate": {
        "kind": "command_regex",
        "pattern": "(^|\\s)npm (install|i|add)\\b",
        "message": "Preference #123: use bun, not npm"
      }
    }
  ]
}
```

Predicate kinds in the first implementation:

- `command_regex`: matched against PostToolUse command input (Bash tool input
  on Claude Code, Codex Bash observe when enabled).
- `commit_trailer_forbidden`: matched against `git commit` command strings for
  forbidden trailer substrings.

Nothing else. New kinds require a spec update.

### Compilation pass (worker side)

1. Select preferences with `status='active'`, reinforcement_count >=
   `rule_compile_min_reinforcement` (default 3), owner scope resolved, and a
   compilable pattern.
2. Compilability is deterministic: a preference qualifies only if its
   structured metadata (or a conservative pattern table for common cases such
   as package-manager choice) yields a predicate; no LLM speculation in the
   first implementation.
3. Drop rules whose source memory is superseded, suppressed, expired, or
   deleted. Contradictory predicates: keep the rule with the newest source
   memory, log the conflict.
4. Write the artifact atomically (temp file + rename).

### Hook evaluation

- PostToolUse: load artifact (mtime-cached in process), evaluate predicates
  against the tool command, append warning text to hook output on match.
- Block action: only honored when the rule has `"action": "block"` set via
  explicit user CLI opt-in; hook returns the host's blocking exit contract.
- Evaluation errors are caught, logged at error level once per session, and
  never propagate.

### CLI

```text
remem rules list [--project <path>]
remem rules disable <rule_id>
remem rules enable <rule_id>
remem rules set-action <rule_id> warn|block
```

Disable is stored in the artifact (worker preserves user overrides across
recompiles by rule_id).

### Doctor

Report artifact presence, rule count, compiled_at age, and last evaluation
error.

## Product-to-Test Mapping

| Product invariant | Implementation area | Verification |
| --- | --- | --- |
| P1 eligibility gating | worker compile pass | unit test: below-threshold preference not compiled |
| P2 provenance | artifact schema | unit test: every rule has source_memory_id |
| P3 deterministic eval | hook evaluator | unit test: same input, same verdict; no DB handle in evaluator |
| P4 warn default | compile pass | unit test: compiled action is warn unless user override exists |
| P5 supersession removal | compile pass | test: soft_supersede source, recompile, rule gone |
| P6 CLI round-trip | rules CLI | test: disable persists across recompile |
| P7 fail open | hook evaluator | test: corrupt artifact -> hook succeeds, error logged |

## Data Flow

Preferences (SQLite) -> worker compile pass -> rules artifact (JSON) ->
hook evaluator (read-only) -> warning text in hook output. User overrides flow
CLI -> artifact -> preserved by recompiles. No hook-side writes.

## Alternatives Considered

- Evaluate rules directly from SQLite in hooks: rejected; adds DB open cost and
  a write-lock hazard to the hot path (capture hooks already required a
  hook-safe DB open path in #467).
- LLM-judged compliance at Stop time: rejected for v1; non-deterministic,
  post-hoc rather than preventive, and adds LLM cost per session.
- Compiling into host-native hook config (Claude Code settings hooks):
  rejected; remem must not rewrite high-context host config files.

## Risks

- Security: rule artifact is an instruction-adjacent surface; it is derived
  from reviewed memories only, written atomically, and never contains
  executable code (predicates are data). Block-mode is user-opt-in only.
- Compatibility: Codex without Bash observe cannot enforce command rules;
  doctor must label per-host enforcement capability honestly.
- Performance: regex evaluation per PostToolUse event; bounded by rule count
  (expected < 20); covered by the latency acceptance criterion.
- Maintenance: predicate kinds are a closed set; growth requires spec update.

## Test Plan

- [ ] Unit tests: compile eligibility, conflict resolution, supersession
      removal, artifact atomicity, evaluator determinism, fail-open.
- [ ] Integration test: end-to-end fixture (preference reinforced 3x -> rule
      compiled -> simulated PostToolUse violation -> warning in output).
- [ ] Manual verification: real Claude Code session with a seeded preference;
      confirm warning appears and `remem rules list` shows provenance.

## Rollback Plan

Config flag off disables compilation and evaluation; deleting
`~/.remem/compiled_rules/` removes all enforcement instantly. No schema
migration is required for rollback (artifact is derived state).
