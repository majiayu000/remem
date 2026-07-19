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
- GH671-T3 adds canonical evidence-backed reinforcement that counts each event
  set once, carries only disjoint evidence across same-predicate replacements,
  and clears stale confidence/provenance on opposing direct saves or cleanup
  rewrites; same-topic direct saves remain isolated by memory type; a closed
  classifier for package-manager and commit-trailer
  predicates; persisted low-risk, source-trust, and review eligibility;
  project-over-global conflict precedence; lifecycle-triggered non-lossy jobs
  plus periodic convergence sweeps; same-predicate override transfer across
  candidate and cleanup supersession; stable diagnostics and artifacts; and
  worker-only artifact writes.
- The current tree also contains rule CLI management, pre-execution Claude
  hook dispatch with honest Codex capability reporting, doctor diagnostics,
  repeated-correction fixtures, and measured hook-latency evidence. #837 at
  merge commit `4d5eafa9b217950b91e8cb46c20c52ce3d9de4a8` supplies management and
  warn-mode round-trip evidence. #839 exact head
  `905a55f7219459dd7b33a1805f0d4da27a97622f`, merged as
  `f612b4a1ec4558ed6d2df85699cefb42109bdf7c`, supplies Claude Code
  PreToolUse and supported-host block persistence. #840 at merge commit
  `ca1a804c8f8b8889ac8b2ba29f5f1c8522f17884` supplies doctor enforcement
  health evidence. T8a reconciles task status and public documentation; final
  acceptance remains open.
- The current compiler still accepts any non-null `owner_scope` for a global
  row. GH-813 tightens that existing gap to the canonical
  `owner_scope='user'`, `owner_key='user:default'`, no-target combination;
  until that implementation and its exhaustive eligibility matrix land,
  malformed or legacy global ownership is not proven fail-closed, T3/T8 remain
  incomplete, and #671 must stay open.
- Preferences are a first-class memory type (`src/memory/types.rs`), rendered
  as a dedicated section in the SessionStart context block
  (`src/context/render.rs`).
- Lesson/preference metadata carries `reinforcement_count`
  (`src/memory_candidate/apply.rs`).
- Hooks are dispatched through `src/cli/dispatch.rs`; Claude Code installs
  SessionStart, UserPromptSubmit, PreToolUse(Bash), PostToolUse, PreCompact,
  and Stop, including `remem rules eval --host claude-code` on PreToolUse.
  Codex installs SessionStart/Stop with PostToolUse(Bash) opt-in and no
  pre-execution rule evaluator. Claude Code PostToolUse observe remains after
  command execution and cannot provide block-mode enforcement.
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
  with a pre-tool hook. Claude Code installation already adds a `PreToolUse`
  Bash hook that invokes the read-only `remem rules eval --host claude-code`
  path before the command runs.
- `commit_trailer_forbidden`: matched against `git commit` command strings for
  forbidden trailer substrings.

Artifact v2 additionally supports:

- `git_push_force_forbidden`: structurally parses shell command segments and
  Git push arguments. It matches exact `--force`, standalone `-f`, or `f` in a
  valid short-option cluster before the `--` terminator, while honoring `-o`
  and long-option arity. Ordinary positional arguments do not match, but a
  non-deletion refspec with a leading `+` does because Git treats that prefix
  as a per-ref force update. Option values, remote names, deletion refspecs,
  and `--force-with-lease` remain non-matches. Shell evaluation uses the Brush
  AST, removes unquoted backslash-newline continuations, traverses assignment
  words, parameter/arithmetic/command substitutions, expandable heredocs,
  static and `builtin eval`, EXIT traps, shell `-c` and stdin payloads,
  `source /dev/stdin` (including persistence of a definite sourced `set --`),
  and statically invoked function bodies, and evaluates
  static brace alternatives. `command`, `env`, and `exec` share one
  command-position normalizer. Quoted or echoed command text and
  uninvoked function definitions remain inert. Static expansion is bounded;
  security-critical static variants remain visible when full materialization
  is capped, and later words or command segments are never discarded. Git
  executable basenames are recognized through static paths, including the
  exact `.exe` suffix used by Git-for-Windows shells and both POSIX and Windows
  command-string path separators; static shell `-c` operands bind `$0` and
  later positional parameters in executed words without leaking the outer
  `$1...` mapping into function bodies, while remaining active for EXIT traps
  and expandable heredoc stdin passed to nested shells; quote characters in an
  unquoted-delimiter heredoc body do not suppress that expansion, while a
  quoted delimiter preserves literal text. Whole unquoted positionals may
  produce zero or multiple argv fields; default and alternative words retain
  quote-aware grouping, known-set error/assignment forms preserve their known
  operand, collection default/alternative forms select from known `$@`/`$*`
  state, exact quoted `"$@"` preserves operand cardinality, static
  non-negative positional slices and substrings retain Bash field and string
  semantics; definite `set --`, argument-bearing `set -`, option-bearing
  positional `set`, and `shift` update the active mapping, while possibly
  executed changes retain prior and
  updated mappings for conservative matching in both whole and concatenated
  words. Possible mappings are evaluated as separate argv alternatives under
  the existing 256-variant ceiling, retaining security-relevant mappings first
  rather than flattening incompatible paths. Positional changes in subshells,
  command substitutions, and
  non-final pipeline processes restore the parent mapping, and aliases resolve
  before builtin positional state. Possible mappings preserve separate
  command-position argv groups and matching suffix paths, so last-option-wins
  flags from mutually exclusive mappings are never concatenated. Static
  non-negative collection slices and positional substrings are materialized,
  with offset zero including `$0`; `shift` advances known mappings and exposes
  static failure to control flow, while argument-bearing `set -` follows
  `set --` assignment semantics. Path-specific positional state changes are
  applied once per correlated mapping; mixed shift success/failure contexts
  select the matching immediate `&&`/`||` branch before rejoining. Stateful
  builtins and function state changes
  such as `trap`, `unset -f`, and function export apply only after alias and
  function resolution. Possible redefinitions preserve all-path alias/function
  presence while retaining every payload/body variant. Each known command,
  alias, function, ordinary fallback, and fallible assignment/redirection setup
  outcome executes against an isolated full shell-state snapshot. Setup failure
  preserves the pre-command state and reports failure; `command`/`builtin`
  wrappers retain known `true`/`false`/`:` status unless a direct function
  shadows the name. Every terminating alternative executes its EXIT traps
  before filtering, and terminated state does not contaminate a continuing
  path while its executable segments remain visible.
  Explicit sourced-file arguments receive their own positional scope, and
  stdin-reading shells bind post-option operands in their child scope.
  Expandable heredoc stdin finishes parent-side positional expansion before a
  child `-c` context is installed. Nested command substitutions and arithmetic
  source use their own syntax context, and function-definition names remain
  unexpanded. Positional command names retain provenance so assignment and
  alias recognition are not rerun after expansion, and here-string positionals
  preserve embedded source newlines. Mixed `builtin`/`command` wrappers and
  the `builtin --` option terminator share the same static builtin
  normalization; force and mirror
  boolean options use Git's last-option-wins behavior (including mirror
  abbreviations); and branches proven unreachable by bare static
  `true`/`false`/`:` guards are not evaluated across `&&`/`||` and `if`/`elif`.
  Function definitions follow Bash subshell, pipeline, shadowing, and static
  `unset -f` state; a function named `unset` resolves before builtin-like state
  mutation, while explicit `builtin unset` retains builtin semantics;
  explicitly exported functions alone enter child Bash,
  while other child shells start empty. Shell `-n`/`noexec` payloads remain
  inert. Shell `-s` and nested static shells inherit the effective final fd-0
  payload under Bash redirection semantics. `env -S` performs bounded argv
  splitting without interpreting shell separators or options that occur after
  its first assignment operand; documented GNU signal options remain wrappers.
  Every materialized or summarized brace-expansion stage remains capped at 256
  segments while preserving one-command argv order and semantically forcing
  short clusters, mirror abbreviations, and force refspecs. Git delete mode
  keeps leading-plus ref names non-forcing.

Nothing else. Further kinds require a spec update.

### Compilation pass (worker side)

1. Evaluate a typed, closed eligibility policy before classification. The
   policy requires memory type `preference`; active and unexpired lifecycle;
   `project` scope with `owner_scope='repo'` and
   `COALESCE(NULLIF(target_project, ''), NULLIF(owner_key, ''), project)` equal
   to the current project, or `global` scope with `owner_scope='user'`,
   `owner_key='user:default'`, and no project target; source trust in
   `local_tool_output`, `repo_file`, or
   `user_prompt`; machine-checkable reinforcement at or above
   `rule_compile_min_reinforcement` (default 3); reinforcement risk `low`;
   originating candidate risk `low`; candidate review status in `approved`,
   `edited`, or `auto_promoted`; successful policy evaluation; and no matching
   `active` memory/topic-key/entity/pattern suppression. Candidate and
   reinforcement risk are separate inputs. Unknown database values, malformed
   suppression state, and unclassified enum variants fail closed with a
   diagnostic. SQL remains
   parameterized and supplies the fields/range; it is not the only expression
   of the safety contract.
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

- PreToolUse (Claude Code Bash): each enabled, valid short-lived `remem rules
  eval` CLI invocation reads and parses the artifact from disk, evaluates
  predicates against the command before execution, and returns the host's
  warning or blocking contract on match. There is no in-process artifact-cache
  contract.
- PostToolUse observe remains capture-only; it may record violations for
  diagnostics but must not be the enforcement path for warning/block behavior.
- Codex command rules are reported as unsupported for enforcement until Codex
  exposes a pre-execution Bash hook. `set-action <rule_id> block` without
  `--host claude-code`, including `--host codex-cli`, returns an error instead
  of implying protection exists.
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
remem rules set-action <rule_id> warn [--host claude-code|codex-cli]
remem rules set-action <rule_id> block --host claude-code
```

Disable/enable and action overrides are stored in SQLite (for example a
`rule_overrides` table keyed by rule_id plus project scope). The worker emits
the merged result into the derived artifact; deleting or regenerating the
artifact cannot revert a user override. `--host` is optional for `warn` and
does not gate that action; `block` requires the explicit
`--host claude-code` capability assertion.

### Doctor

Report whether compilation is enabled, artifact presence/validity and rule
count, compile status/time/error, the latest project/global evaluation error,
and per-host enforcement capability. Human and JSON output must not expose rule
payloads.

## Product-to-Test Mapping

| Product invariant | Implementation area | Verification |
| --- | --- | --- |
| P1 eligibility gating | typed eligibility policy plus worker compile pass | one complete positive fixture; table-driven single-dimension negatives for type, lifecycle, expiry, scope/owner/project, trust, machine-checkability, threshold, independent reinforcement/candidate risk, review status, and suppression; unknown/closed-enum coverage; critical cross-state cases |
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
- Performance: predicate evaluation per pre-execution Bash event; bounded by
  rule count (expected < 20). The release benchmark gates on enabled p95
  `<= 15.0 ms` and enabled-minus-disabled p95 delta `<= 1.0 ms`; MAD is
  retained only as informational output.
- Artifact compatibility: new compilations emit schema v2 with ASCII-delimited
  `command_regex` patterns evaluated by `regex-lite`. Schema v1 remains readable
  and retains its original Unicode `regex` semantics until the worker replaces
  the derived artifact. The v2 classifier recognizes only closed
  package-manager, commit-trailer, and exact low-risk forbidden-command
  directives. Package-manager `command_regex` patterns use `regex-lite`; the
  forbidden-command allowlist contains only `git push --force` and compiles to
  the structural `git_push_force_forbidden` predicate.
- Project-root marker discovery skips the fast path whenever explicit Git
  layout, discovery controls, command-scope config injection, or local/default
  Git config can change worktree resolution. This includes
  `GIT_CEILING_DIRECTORIES`, `GIT_CONFIG_PARAMETERS`, paired
  `GIT_CONFIG_COUNT`/`GIT_CONFIG_KEY_*` inputs, local `core.worktree`, and
  global/system/XDG config containing worktree-affecting values or includes.
  Gitfile, symlink, and filesystem-device-boundary semantics remain owned by
  `git rev-parse`; only a real `.git` directory with a conservatively validated
  plain layout may use the marker fast path. Plain config stays on that fast
  path so hooks do not restore an unconditional Git subprocess.
- Latency evidence compares repeated interleaved CLI subprocess cohorts. Pass
  requires both fixed budgets: enabled p95 `<= 15.0 ms` and
  enabled-minus-disabled p95 delta `<= 1.0 ms`. Median absolute deviation is
  retained as informational output and does not affect pass/fail. The fresh
  final-head fixed-budget artifact measured baseline p95 `7.627417 ms`, enabled
  p95 `7.905666 ms`, delta `0.278249 ms`, complex-AST p95 `7.921000 ms`, and
  MAD `0.313583 ms`; it passes both fixed budgets.
- Maintenance: predicate kinds are a closed set; growth requires spec update.

## Test Plan

- [x] Existing unit tests: basic compile eligibility, conflict resolution,
      supersession removal, artifact atomicity, evaluator determinism, and
      fail-open behavior.
- [ ] Exhaustive eligibility contract tests: one eligible baseline,
      independently mutable candidate and reinforcement risk, one negative per
      dimension, unknown values, closed-enum completeness, and critical
      cross-state cases. Tests remain behavior-based and do not snapshot the
      SQL/WHERE text.
- [x] Integration test: end-to-end fixture (preference reinforced 3x -> rule
      compiled -> simulated PreToolUse Bash violation -> warning/block before
      execution).
- [x] CLI management tests: #837 covers provenance listing and the
      management/warn round trip across artifact deletion and worker rebuild
      (`src/rules/management/tests.rs:82-138`). The shared
      unsupported-pre-execution rejection path
      (`src/rules/management/tests.rs:142-174`)
      proves no override/job persistence without separately exercising host
      None and Codex CLI values. #839 exact head
      `905a55f7219459dd7b33a1805f0d4da27a97622f` (merged as
      `f612b4a1ec4558ed6d2df85699cefb42109bdf7c`) covers supported Claude block
      persistence (`src/rules/management/tests.rs:177-204`). Existing compiler
      coverage reconstructs stored overrides
      (`src/rules/compiler/tests.rs:380-399`).
- [x] Doctor tests: #840 covers human/JSON artifact and compile health, latest
      evaluation diagnostics, Claude/Codex capability reporting, recovery, and
      payload privacy.
- [ ] Manual verification: real Claude Code session with a seeded preference;
      confirm warning appears and `remem rules list` shows provenance.

## Rollback Plan

Config flag off disables compilation and evaluation; deleting
`~/.remem/compiled_rules/` removes all enforcement instantly. No schema
migration is required for rollback (artifact is derived state).
