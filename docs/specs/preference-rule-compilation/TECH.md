# Preference Rule Compilation Technical Spec

Status: Current contract
Date: 2026-07-11

Tracking:
- Spec/tracking issue: #671
- Related umbrella: #383

## Existing Implementation Facts

- Phase 1 state foundation is implemented: runtime config defaults keep rule
  compilation disabled by default, SQLite stores preference reinforcement,
  rule override, and rule diagnostic state, and schema drift/convergence tests
  cover those tables and indexes.
- GH671-T2 adds the derived artifact foundation: versioned JSON structs,
  the closed v1 predicate enum, a pure in-memory evaluator, fail-open artifact
  loading for missing/corrupt/unsupported artifacts, stable project artifact
  paths, and atomic artifact writes.
- GH671-T3 adds canonical evidence-backed reinforcement for identical and
  same-predicate replacements without transferring confidence across opposing
  preferences; a closed classifier for package-manager and commit-trailer
  predicates; persisted low-risk, source-trust, and review eligibility;
  project-over-global conflict precedence; lifecycle-triggered non-lossy jobs
  plus periodic convergence sweeps; same-predicate override transfer; stable
  diagnostics and artifacts; and worker-only artifact writes. Hook dispatch,
  rule CLI, doctor output, fixtures, and latency evidence are still pending.
- Preferences are a first-class memory type (`src/memory/types.rs`), rendered
  as a dedicated section in the SessionStart context block
  (`src/context/render.rs`).
- Lesson/preference metadata carries `reinforcement_count`
  (`src/memory_candidate/apply.rs`).
- Hooks are dispatched through `src/cli/dispatch.rs`; Claude Code fires
  SessionStart, UserPromptSubmit, PostToolUse, PreCompact, and Stop today;
  Codex fires SessionStart/Stop with PostToolUse(Bash) opt-in. The existing
  Claude Code PostToolUse observe hook is after command execution and cannot
  provide block-mode enforcement.
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

Derived file per project at
`<data_dir>/compiled_rules/<project-hash>.json`, where `<data_dir>` is the
resolved `REMEM_DATA_DIR` / `db::absolute_data_dir()` location:

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
        "message": "Command violates a compiled package-manager preference"
      }
    }
  ]
}
```

Predicate kinds in the first implementation:

- `command_regex`: matched against pre-execution Bash command input on hosts
  with a pre-tool hook. Claude Code support requires adding a `PreToolUse`
  Bash hook that invokes a read-only `remem rules eval` path before the
  command runs.
- `commit_trailer_forbidden`: matched against `git commit` command strings for
  forbidden trailer substrings.

Nothing else. New kinds require a spec update.

### Compilation pass (worker side)

1. Select preferences with `status='active'`, reinforcement_count >=
   `rule_compile_min_reinforcement` (default 3), owner scope resolved,
   originating candidate `risk_class='low'`, a persisted source trust class of
   `local_tool_output`, `repo_file`, or `user_prompt`, and a compilable pattern.
2. Compilability is deterministic: a preference qualifies only if its
   structured metadata (or a conservative pattern table for directed
   npm/yarn/bun/pnpm choices and forbidden commit trailers) yields a predicate;
   no LLM speculation in the first implementation. One source may produce
   multiple trailer predicates.
3. Drop rules whose source memory is superseded, suppressed, expired, or
   deleted. Project-scoped predicates take precedence over global predicates;
   within the same scope the newest source wins and the conflict is logged.
4. Load user overrides from canonical SQLite state, transfer source-bound
   overrides when a replacement memory becomes authoritative, then write the
   artifact atomically (temp file + rename). The artifact is never the source of truth
   for disabled/enabled/action override state. Artifact messages use static
   predicate-kind wording and never copy arbitrary preference text.
5. Preference apply, suppression, unsuppression, expiry, supersession, and
   deletion schedule compilation jobs. A mutation arriving while compilation
   is processing leaves one pending successor so canonical-state changes are
   not lost.

### Hook evaluation

- PreToolUse (Claude Code Bash): load artifact (mtime-cached in process),
  evaluate predicates against the command before execution, and return the
  host's warning or blocking contract on match.
- PostToolUse observe remains capture-only; it may record violations for
  diagnostics but must not be the enforcement path for warning/block behavior.
- Codex command rules are reported as unsupported for enforcement until Codex
  exposes a pre-execution Bash hook; `set-action ... block` returns an error
  for Codex-only projects instead of implying protection exists.
- Block action: only honored when the rule has `"action": "block"` set via
  explicit user CLI opt-in and the current host supports pre-execution
  enforcement.
- Evaluation errors are caught, logged at error level once per session, and
  never propagate.

### CLI

```text
remem rules list [--project <path>]
remem rules disable <rule_id>
remem rules enable <rule_id>
remem rules set-action <rule_id> warn|block
```

Disable/enable and action overrides are stored in SQLite (for example a
`rule_overrides` table keyed by rule_id plus project scope). The worker emits
the merged result into the derived artifact; deleting or regenerating the
artifact cannot revert a user override.

### Doctor

Report artifact presence, rule count, compiled_at age, and last evaluation
error.

## Product-to-Test Mapping

| Product invariant | Implementation area | Verification |
| --- | --- | --- |
| P1 eligibility gating | worker compile pass | unit tests: threshold, activity, risk, trust, machine-checkability, and scope precedence |
| P2 provenance | artifact schema | unit test: every rule has source_memory_id |
| P3 deterministic eval | hook evaluator | unit test: same input, same verdict; no DB handle in evaluator |
| P4 warn default | compile pass | unit test: compiled action is warn unless user override exists |
| P5 supersession removal | compile pass | test: soft_supersede source, recompile, rule gone |
| P6 CLI round-trip | rules CLI + SQLite overrides | test: disable/action override persists across artifact deletion and recompile |
| P7 fail open | hook evaluator | test: corrupt artifact -> hook succeeds, error logged |
| P8 pre-execution enforcement | install + evaluator | test: Claude PreToolUse Bash hook blocks before command; PostToolUse-only path cannot claim enforcement |

## Data Flow

Preferences + rule overrides (SQLite) -> worker compile pass -> rules artifact
(JSON) -> pre-execution hook evaluator (read-only) -> warning/block result in
hook output. User overrides flow CLI -> SQLite -> next artifact build. No
hook-side writes.

## Alternatives Considered

- Evaluate rules directly from SQLite in hooks: rejected; adds DB open cost and
  a write-lock hazard to the hot path (capture hooks already required a
  hook-safe DB open path in #467).
- LLM-judged compliance at Stop time: rejected for v1; non-deterministic,
  post-hoc rather than preventive, and adds LLM cost per session.
- PostToolUse-only enforcement: rejected; commands have already run by then,
  so it cannot satisfy block-mode semantics for package-manager or forbidden
  command rules.
- Compiling into host-native hook config (Claude Code settings hooks):
  rejected; remem must not rewrite high-context host config files.

## Risks

- Security: rule artifact is an instruction-adjacent surface; it is derived
  from reviewed memories only, written atomically, and never contains
  executable code (predicates are data). Block-mode is user-opt-in only.
- Compatibility: Codex and any host without pre-execution Bash hooks cannot
  enforce command rules; doctor must label per-host enforcement capability
  honestly and CLI must reject unsupported block-mode claims.
- Performance: regex evaluation per pre-execution Bash event; bounded by rule count
  (expected < 20); covered by the latency acceptance criterion.
- Maintenance: predicate kinds are a closed set; growth requires spec update.

## Test Plan

- [x] Unit tests: compile eligibility, conflict resolution, supersession
      removal, artifact atomicity, evaluator determinism, fail-open.
- [ ] Integration test: end-to-end fixture (preference reinforced 3x -> rule
      compiled -> simulated PreToolUse Bash violation -> warning/block before
      execution).
- [ ] Manual verification: real Claude Code session with a seeded preference;
      confirm warning appears and `remem rules list` shows provenance.

## Rollback Plan

Config flag off disables compilation and evaluation; deleting
`~/.remem/compiled_rules/` removes all enforcement instantly. No schema
migration is required for rollback (artifact is derived state).
