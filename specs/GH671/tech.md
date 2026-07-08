# Tech Spec

## Linked Issue

GH-671

## Product Spec

Product: `product.md`

Authoritative contract:
`docs/specs/preference-rule-compilation/PRODUCT.md` and
`docs/specs/preference-rule-compilation/TECH.md`.

## Codebase Context

| Area | Files | Current behavior | Why relevant |
| --- | --- | --- | --- |
| Preference memory | `src/memory/types.rs`, `src/context/render.rs`, `src/context/sections/lessons.rs` | Preferences and lessons are stored and rendered into SessionStart context as prose. | Compiled rules derive from the same authoritative memory layer and must not replace injection. |
| Reinforcement metadata | `src/memory_candidate/apply.rs`, `src/memory/lesson.rs`, `src/migrations/v017_memory_lessons.sql` | Existing `memory_lessons` reinforcement state is lesson-specific; active `preference` rows do not yet have canonical persisted reinforcement counts. | Eligibility depends on repeated corrections meeting a configured threshold, so implementation must add or wire real preference reinforcement state before compiling preferences. |
| Suppression and lifecycle | `src/memory/suppression.rs`, `src/memory/lifecycle.rs`, `src/migrations/v051_memory_suppressions_feedback.sql` | Memories can be suppressed or soft-superseded. | Derived rules must disappear when source memories stop being authoritative. |
| Worker path | `src/worker.rs`, `src/db_job.rs`, `src/summarize.rs` | Background work runs outside interactive hooks. | Rule compilation belongs off the hook hot path. |
| Hook dispatch and install | `src/cli/dispatch.rs`, `src/install.rs`, hook configuration tests | Hooks inject context and capture observations; Claude PostToolUse is post-execution and Codex command observe is opt-in. | Warning/block enforcement needs a pre-execution Claude Bash hook and honest unsupported behavior elsewhere. |
| CLI commands | `src/cli/types.rs`, `src/cli/dispatch.rs`, `src/cli/actions/` | User-facing commands manage memory and diagnostics. | `remem rules` must expose list, disable, enable, and action override flows. |
| Doctor | `src/doctor/`, `src/doctor/tests.rs` | Doctor reports runtime and data health through human and JSON output. | Compile/evaluation status and host capability must be visible, not silently degraded. |

## Proposed Design

Phase 1 implementation status: `SP671-T1` is implemented as state-only
foundation. `SP671-T2` adds versioned derived artifact structs, the closed v1
predicate enum, pure evaluator, fail-open artifact loading, stable artifact
paths, and atomic writes. Runtime compilation, CLI rule management, hook
dispatch, doctor reporting, fixtures, and latency evidence remain pending.

- Add a `rules` module with a versioned artifact schema, closed predicate enum,
  pure evaluator, compiler, and atomic artifact writer.
- Add a migration for canonical override and diagnostic state, for example
  `rule_overrides` keyed by project scope plus `rule_id`, and a compact
  compile/evaluation status table or sidecar whose values doctor can report.
- Add canonical preference reinforcement state or an equivalent typed metadata
  path for active `preference` memories. The compiler must not infer repeated
  preference eligibility from lesson-only `memory_lessons` rows.
- Add config for `rule_compilation_enabled` and
  `rule_compile_min_reinforcement` with disabled-by-default rollout behavior
  and default threshold `3`.
- Compile only deterministic v1 predicates:
  `command_regex` and `commit_trailer_forbidden`. New predicate kinds require
  a spec update.
- Store derived artifacts under
  `<data_dir>/compiled_rules/<project-hash>.json`. SQLite remains canonical;
  artifacts are regenerated output.
- Run artifact compilation and writes only from the background worker. Hooks
  only load and evaluate artifacts.
- Add `remem rules list [--project <path>]`, `disable`, `enable`, and
  `set-action warn|block`. Overrides update SQLite and become effective after
  the next artifact build.
- Add a Claude Code `PreToolUse` Bash hook for command evaluation before
  execution. Keep PostToolUse observe capture-only. Report Codex command
  enforcement as unsupported until Codex exposes a pre-execution command hook.
- Add doctor output for artifact presence, rule count, last compile time, last
  compile/evaluation error, and host enforcement capability.

## Product-to-Test Mapping

| Product invariant | Implementation area | Verification |
| --- | --- | --- |
| P1 eligibility | compiler query and predicate classifier | Unit tests for threshold, inactive, ambiguous, low-risk, and scope cases |
| P2 provenance | artifact schema and compiler | Schema/unit test asserting provenance fields on every rule |
| P3 deterministic local evaluation | pure evaluator | Unit test same input/same verdict; evaluator has no DB write path |
| P4 warning default and opt-in block | compiler plus override merge | Unit tests default action, override action, unsupported block rejection |
| P5 source lifecycle removal | compiler source query | Supersede, suppress, expire, and delete fixtures remove rules |
| P6 persistent overrides | migration and CLI | CLI round-trip persists across artifact deletion and recompile |
| P7 fail-open diagnostics | evaluator and doctor | Corrupt/missing artifact hook tests plus doctor JSON/human assertions |
| P8 host capability honesty | install, CLI, doctor | Claude PreToolUse test and Codex unsupported block-mode test |

## Data Flow

```text
preferences + preference reinforcement state + suppressions + rule_overrides
  -> worker compile pass
  -> atomic compiled_rules/<project-hash>.json
  -> pre-execution hook rule eval
  -> warning/block result or fail-open diagnostic
  -> doctor visibility
```

The hook path reads the artifact and writes no database rows. CLI override
commands write SQLite state; the next compile pass merges overrides into the
derived artifact.

## Alternatives Considered

- Read SQLite directly from hooks: rejected because hook latency and lock
  hazards are worse than reading a small derived artifact.
- LLM compliance checks in hooks or Stop: rejected because the feature must be
  deterministic and preventive.
- PostToolUse-only enforcement: rejected because the command has already run
  and cannot satisfy block-mode semantics.
- Host-native generated hook policy files: rejected because remem should not
  rewrite high-context host config as the source of rule truth.

## Risks

- Security: Rule artifacts are instruction-adjacent data. They must contain no
  executable code, no secrets, and only closed predicate kinds.
- Compatibility: Codex and hosts without pre-execution hooks cannot enforce
  command block mode; CLI and doctor must say that plainly.
- Performance: Hook evaluation must stay bounded by a small rule count and
  avoid DB/network/LLM work.
- Maintenance: Predicate growth can become a hidden rules engine; require spec
  updates for new predicate kinds.

## Test Plan

- [ ] Unit tests: config parsing, compiler eligibility, predicate classifier,
      preference reinforcement state, conflict resolution, source lifecycle
      removal, artifact atomicity, evaluator determinism, and fail-open
      behavior.
- [ ] CLI tests: `rules list`, `disable`, `enable`, and `set-action` across
      artifact deletion and recompile.
- [ ] Hook integration tests: simulated Claude PreToolUse Bash warning/block,
      PostToolUse capture-only behavior, and Codex unsupported enforcement.
- [ ] Doctor tests: human and JSON output for count, compile time, host
      capability, and last error.
- [ ] Fixture/eval tests: repeated-correction scenarios and hook latency
      benchmark.
- [ ] Existing gates: `cargo fmt --check`, `cargo check`, focused tests, and
      `cargo test` before merge readiness.

## Rollback Plan

Disable the config flag to stop compilation and evaluation. Deleting
`<data_dir>/compiled_rules/` removes derived enforcement artifacts immediately.
If code rollback is needed, leave inert override/diagnostic migration tables in
place; they are canonical user state but are ignored when the feature is off.
