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
| Reinforcement metadata | `src/memory_candidate/apply.rs`, `src/memory/preference/reinforcement.rs`, `src/migrations/v062_preference_rule_state.sql`, `src/migrations/v065_preference_reinforcement.sql` | Active preferences carry canonical, evidence-backed reinforcement counts plus machine-checkable, risk, and source-evidence state; each event set counts once, disjoint evidence and overrides merge only across the same safe predicate, and opposing direct saves or cleanup rewrites clear stale rule provenance. | Eligibility reads real repeated-correction state without allowing duplicate evidence, contradictory content, or unsafe preferences to inherit confidence. |
| Suppression and lifecycle | `src/memory/suppression.rs`, `src/memory/lifecycle.rs`, `src/migrations/v051_memory_suppressions_feedback.sql` | Memories can be suppressed or soft-superseded. | Derived rules must disappear when source memories stop being authoritative. |
| Worker path | `src/worker.rs`, `src/db_job.rs`, `src/summarize.rs` | Background work runs outside interactive hooks. | Rule compilation belongs off the hook hot path. |
| Hook dispatch and install | `src/cli/dispatch.rs`, `src/install.rs`, hook configuration tests | Hooks inject context and capture observations; Claude PostToolUse is post-execution and Codex command observe is opt-in. | Warning/block enforcement needs a pre-execution Claude Bash hook and honest unsupported behavior elsewhere. |
| CLI commands | `src/cli/types.rs`, `src/cli/dispatch.rs`, `src/cli/actions/` | User-facing commands manage memory and diagnostics. | `remem rules` must expose list, disable, enable, and action override flows. |
| Doctor | `src/doctor/`, `src/doctor/tests.rs` | Doctor reports runtime and data health through human and JSON output. | Compile/evaluation status and host capability must be visible, not silently degraded. |

## Proposed Design

Phase 1 task-ledger status: `SP671-T1` through `SP671-T3` and `SP671-T5`
through `SP671-T7` are recorded as implemented.
The state foundation, versioned artifact/evaluator, canonical preference
reinforcement, deterministic compiler, lifecycle-triggered non-lossy enqueue
path plus periodic convergence sweeps, same-predicate override transfer,
persisted low-risk/source-trust/review eligibility, project-over-global
precedence, stable diagnostics, worker-only artifact writes, CLI rule
management, hook dispatch, doctor reporting, repeated-correction fixtures, and
measured hook-latency evidence are present. The task ledger still leaves
`SP671-T4` unchecked; `SP671-T8` owns its status reconciliation along with
documentation and final acceptance.

GH-813 identified one remaining T3 eligibility correction: the current global
branch accepts any non-null `owner_scope`; it must require the canonical
`user` / `user:default` / no-target combination before the closed eligibility
contract is complete.

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
- Express the P1 eligibility boundary as a typed, closed policy. SQL remains
  parameterized and supplies the relevant rows/fields, while the policy treats
  memory type, lifecycle/expiry, project/repo ownership with resolved-target
  precedence `target_project` then `owner_key` then legacy `project`, the exact
  global/user ownership combination, source trust,
  machine-checkability, threshold, reinforcement risk, originating candidate
  risk, candidate review status, and suppression/policy state as independent
  inputs. Policy evaluation must succeed and find no matching active memory/
  topic-key/entity/pattern suppression. Unknown values and malformed policy
  state fail closed with diagnostics; candidate and reinforcement risk must not
  share one test fixture field.
- Continue accepting the deterministic v1 predicates `command_regex` and
  `commit_trailer_forbidden`. Artifact v2 adds the closed
  `git_push_force_forbidden` predicate; other new kinds require a spec update.
- Emit artifact schema v2 for new compilations. V2 command regexes use an
  ASCII-delimited closed grammar and `regex-lite` so short-lived hook processes
  do not pay Unicode-regex compilation cost. Continue accepting and evaluating
  v1 artifacts with the original `regex` engine so upgrades preserve existing
  Unicode semantics until the worker regenerates the derived artifact.
- The v2 classifier may emit `git_push_force_forbidden` for the exact, closed
  low-risk directive `git push --force`. Evaluation uses the existing shell
  tokenizer and a typed Git-push argument parser. Unquoted newlines and command
  groups form executable segments, while quoted or echoed command text stays
  inert, unquoted backslash-newline is removed as line continuation, and
  heredoc bodies are excluded. Exact `--force`, standalone `-f`, `f` in valid
  short-option clusters, and a non-deletion leading-`+` refspec match; ordinary
  positional arguments do not. The parser honors the `--` terminator and
  option arity so option values, deletions, remote names,
  `--force-with-lease`, and arbitrary natural-language commands fail closed.
- The project-root marker fast path is used only when Git discovery has no
  environment override and the nearest `.git` marker is a plain worktree
  layout. Explicit layouts, discovery controls such as
  `GIT_CEILING_DIRECTORIES`, command-scope config injection, local/default
  global/system/XDG worktree-affecting Git config, and malformed inner markers
  delegate to Git so project identity matches Git's own toplevel or fails
  closed instead of falling through to a parent marker. Plain config keeps the
  no-subprocess marker path.
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
| P1 eligibility | typed eligibility policy, compiler query, and predicate classifier | One complete positive fixture; table-driven single-dimension negatives including independently mutable reinforcement/candidate risk; exact review/trust allowlists; unknown/closed-enum completeness; critical cross-state cases; no SQL text snapshots |
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

- [x] Unit tests: config parsing, compiler eligibility, predicate classifier,
      preference reinforcement state, conflict resolution, source lifecycle
      removal, artifact atomicity, evaluator determinism, and fail-open
      behavior.
- [ ] Exhaustive P1 matrix: one eligible baseline, independent candidate and
      reinforcement risk, one negative per eligibility dimension, exact
      owner/trust/review allowlists, unknown values, closed-enum completeness,
      policy failure, and critical cross-state cases without SQL text snapshots.
- [ ] CLI tests: `rules list`, `disable`, `enable`, and `set-action` across
      artifact deletion and recompile.
- [x] Hook integration tests: simulated Claude PreToolUse Bash warning/block,
      PostToolUse capture-only behavior, and Codex unsupported enforcement.
- [ ] Doctor tests: human and JSON output for count, compile time, host
      capability, and last error.
- [x] Fixture/eval tests: repeated-correction scenarios and hook latency
      benchmark.
- [ ] Existing gates: `cargo fmt --check`, `cargo check`, focused tests, and
      `cargo test` before merge readiness.

## Rollback Plan

Disable the config flag to stop compilation and evaluation. Deleting
`<data_dir>/compiled_rules/` removes derived enforcement artifacts immediately.
If code rollback is needed, leave inert override/diagnostic migration tables in
place; they are canonical user state but are ignored when the feature is off.
