# Tech Spec

## Linked Issue

GH-860

## Product Spec

[`product.md`](product.md)

## Codebase Context

| Area | Files | Current behavior | Why relevant |
| --- | --- | --- | --- |
| Static command collection | `src/rules/evaluator/bash_ast.rs`, `src/rules/evaluator/bash_ast/command_resolution.rs`, `src/rules/evaluator/bash_ast/alternative_state.rs` | `collect_static_tokens` resolves normalized command variants and recursively parses static shell `-c` payloads while shell-state alternatives are isolated and merged | Resolution ordering, fallible setup, wrapper status, termination, and complete state correlation determine whether executable paths remain visible |
| Shell recognition and `-c` parsing | `src/rules/evaluator/bash_ast/static_execution.rs:445` | `static_shell_command_payload` returns only the literal command string; `shell_name` accepts five suffix-free basenames | `.exe` recognition and the shell argument boundary belong in this normalization layer |
| Positional expansion | `src/rules/evaluator/bash_ast.rs`, `src/rules/evaluator/bash_ast/shell_state.rs`, `src/rules/evaluator/bash_ast/function_args.rs`, `src/rules/evaluator/bash_ast/function_args/quoting.rs`, `src/rules/evaluator/bash_ast/stdin_payload.rs` | Function-body expansion already handles quoted/unquoted positional parameters with bounded default expansion, but assumes function arguments where `$1` starts at the first operand and `$0` is unavailable | Shell `-c` needs an explicit Bash `$0`/`$1...` context that yields to function-call arguments inside function bodies and remains active for deferred traps and expandable heredocs |
| Structural regression fixtures | `src/rules/evaluator/tests/git_execution.rs:398` | Paired block/allow tables cover static shell state, indirect execution, function arguments, and false-block precision | The three GH-860 behaviors fit the existing table-driven evidence style |
| Authoritative rule contract | `docs/specs/preference-rule-compilation/TECH.md:114` | Artifact-v2 documents shell payloads, function state, positional function arguments, and bounded evaluation | The shipped contract must name the additional `.exe`, shell-`-c`, and shadowed-builtin semantics |
| Release/version gates | `scripts/ci/check_version_bump.py`, `scripts/ci/check_plugin_version_sync.py` | Any `src/` change requires a package version above the base and synchronized release surfaces | This PR must stage one synchronized patch version |

## Proposed Design

### 1. Normalize supported `.exe` shell basenames

Keep `shell_name` as the single recognition point. Resolve the static command's
basename across both POSIX and Windows command-string separators and accept the
existing closed shell set both without a suffix and with one exact lowercase
`.exe` suffix. Return the normalized suffix-free shell name so
`static_shell_is_bash` preserves exported-function inheritance for `bash.exe`.
Do not perform filesystem lookup, case folding, or substring matching.

### 2. Expand statically known shell `-c` arguments

Replace the literal-only shell payload helper with a helper that returns the
command string plus the static operands after it. Refactor the existing
function positional expander into a string-source helper with an explicit
`$0` value and positional-argument slice:

- function calls preserve the existing unresolved outer-shell `$0` behavior
  and pass their current argument slice as `$1...`;
- shell `-c` passes the first operand after the command string as `$0`, and
  the remaining operands as `$1...`;
- absent `$0` is represented as an unknown/empty shell name without shifting
  `$1`;
- missing positional operands keep the existing conservative empty expansion;
  dynamic sentinel operands do not become fabricated static commands.
- an entire unquoted positional word may materialize multiple static argv
  fields, while function-definition names are never parameter-expanded;
- nested command substitutions are parsed from their own source before
  applying the inherited positional context, and arithmetic positionals are
  expanded as arithmetic source rather than shell-quoted argv.
- zero-field unquoted expansions remove the word, default/alternative words
  preserve their own quote-aware field grouping, and `${n+word}` / `${n:+word}`
  select statically when the operand state is known; known-set `${n?word}`,
  `${n:?word}`, `${n=word}`, and `${n:=word}` forms preserve that operand;
  `${@:-word}` / `${@:+word}` and their non-colon forms select from the known
  collection state;
- definite static `set --` or argument-bearing `set -` replaces `$1...` while
  retaining `$0`, and definite static `shift [n]` advances every active
  argument alternative; recognized `set` options are consumed through the
  positional boundary before that replacement. An uncertain change retains
  both prior and updated
  argument sets so each possible path contributes static fields, including
  positional references concatenated with literal word content; each mapping
  is evaluated as its own command argv rather than flattened with other paths;
- possible mappings reuse `MAX_STATIC_WORD_VARIANTS` as a hard ceiling, with
  security-relevant argument sets retained before non-critical alternatives;
- static non-negative `${@:offset[:length]}` slices preserve selected argument
  cardinality, while `${n:offset[:length]}` applies a bounded Unicode-scalar
  substring before the existing quote-aware field handling;
- exact quoted `"$@"` preserves one field per operand, while exact quoted
  numeric parameters preserve their single-field grouping;
- positional state participates in shell-state snapshots so subshells,
  command substitutions, and non-final pipeline processes restore the parent
  mapping; alias and function calls resolve before builtin `set --` mutation;
- possible mappings materialize separate command-position argv segments rather
  than concatenating alternative commands into one argv; every suffix expands
  against the matching mapping so mutually exclusive last-option-wins flags
  remain on separate evaluated paths;
- bounded non-negative `${@:offset[:length]}` collection slices and
  `${n:offset[:length]}` substrings are evaluated statically; definite `shift`
  advances every known mapping, possible `shift` retains shifted and unshifted
  mappings, `${@:0}` includes `$0`, and argument-bearing `set -` follows
  `set --` assignment semantics; static shift success/failure feeds `&&`/`||`
  reachability without mutating positionals on failure;
- when a positional-state command expands differently on known paths, apply
  one transition per matching argument set and merge the results; keep
  successful and failed shift contexts separate while evaluating the immediate
  `&&`/`||` branch, then merge executed and skipped contexts afterward;
- execute each known command, alias body, function body, ordinary fallback,
  and statically fallible readonly-assignment or redirection setup outcome
  against an isolated full shell-state snapshot. A possible alias or function
  retains an isolated ordinary builtin/external fallback. Setup is branched
  before command resolution, with failure preserving the pre-command snapshot
  and reporting a failing status; plain assignment prefixes not targeting a
  known readonly variable do not create a synthetic failure alternative in
  either top-level or positional child-shell control flow. Function-mode
  `readonly -f` operands do not enter variable readonly state, while
  `readonly -p NAME` retains Bash's variable declaration behavior. Known
  `true`/`false`/`:` status is read after the shared
  `command`/`builtin` wrapper normalizer unless a direct function shadows the
  name. Collect EXIT traps for every terminating alternative before snapshot
  filtering, and discard terminated state from a continuing-state merge while
  preserving its already-collected executable segments;
- expandable heredocs materialize parent positionals before child-shell scope,
  and explicit `source [--] /dev/stdin` arguments temporarily replace `$1...`
  while the sourced body is analyzed; source success/failure is rebound to the
  restored caller positional context before `&&`/`||` continues unless a
  definite sourced `set --` persists a new mapping with Bash semantics.
- stdin-reading shells bind post-option operands as `$1...` in their child
  scope, with the normalized shell basename retained as `$0`.
- command-position words materialized from positional expansion retain a
  bounded provenance marker so assignment-prefix and lexical-alias recognition
  are not rerun after expansion; consumers strip that marker only when reading
  the semantic executable name, including `command`/`env`/`exec` and `env -S`
  normalization;
- here-strings use a no-field-splitting positional expander so embedded
  newlines reach nested stdin parsing as source text.

Parse the literal command string through the existing child-shell scope while
carrying the mapping as collector context. Expand each executed word in that
context, but store function definitions without outer positional replacement;
when a function is invoked, its `$1...` values come from the function call and
only the shell `$0` context remains visible. Execute EXIT traps before restoring
the child shell context, and expand unquoted heredoc payloads in the current
context before a nested shell receives stdin. Use heredoc-specific expansion
semantics so quote characters in an unquoted-delimiter body are literal and do
not suppress `$0/$1...`; quoted-delimiter heredocs remain literal. Quoting,
recursive defaults, default-word quote removal, and materialization bounds
remain owned by the existing positional-expansion implementation.

### 3. Resolve functions before builtin-like state mutation

Determine whether the direct command position names a currently known function
before applying builtin state changes. A plain `unset ...` call resolves to the
function and returns after analyzing its body. Explicit builtin-selection
forms (`builtin unset ...` and the existing `command unset ...` behavior) skip
function resolution and may mutate static function state. The state mutation
helper shall use the shared builtin command-position normalizer, which keeps
peeling valid `builtin` and `command` wrappers in either order so mixed and
repeated wrappers remain deterministic, including the `builtin --` option
terminator before the selected builtin name.

Stateful shell builtins, including `trap`, alias/shopt state, positional
mutation, `unset -f`, and function-export state, are applied only after alias
and function resolution. Explicit builtin-selection forms retain builtin
behavior. An alias or function already present on every path remains known to
be present after a possible redefinition even though its payload/body has
multiple variants. This does not change subshell scopes or dynamic command
resolution.

### 4. Contract and staged version

Add paired positive/negative cases to the existing structural evaluator tests,
update the authoritative technical contract, and stage source version `0.6.9`.
Synchronize Cargo, plugin, npm, MCP server, release manifest, lockfile, and
changelog metadata. The release manifest remains `state: unreleased` with no
assets; this PR does not publish a release.

## Product-to-Test Mapping

| Product invariant | Implementation area | Verification |
| --- | --- | --- |
| B-001 `.exe` shell equivalence | normalized `shell_name` in `src/rules/evaluator/bash_ast/static_execution.rs` | `force_push_rule_recognizes_exe_shell_basenames` covers `bash.exe -c 'git push --force'` → Block |
| B-002 basename-only precision | platform-independent `shell_name` and block/allow fixture tables | `force_push_rule_recognizes_exe_shell_basenames` covers POSIX- and Windows-qualified `bash.exe` → Block and unrelated `notbash.exe` → Allow |
| B-003 shell `-c` positional binding | scoped positional collector context, quote-aware field expansion, separately evaluated bounded positional variants, command provenance, isolated shell-state snapshots, and shell/source payload extraction | focused tests cover `$0`, zero/multi-field `$1`, quoted `"$@"`, command/suffix path alternatives, `${@:0}`, slices/substrings, default/alternative and known-set error/assignment words, definite and uncertain `set --` (including concatenated words and distinct argv alternatives), `set -`, successful/failed `shift` control flow, alias/function and ordinary fallback ordering, fallible setup, wrapper status, terminated-path filtering, child-scope restoration, assignment/alias provenance, function-local and sourced arguments, nested command substitutions, arithmetic source, EXIT traps, here-strings, and parent-versus-child heredoc handoff |
| B-004 missing positional operands | shell payload extraction and positional expansion | `force_push_rule_binds_shell_command_positional_parameters` covers absent and safe `$1`; `force_push_rule_preserves_missing_shell_zero` leaves `${0:-git}` unknown rather than fabricating `git` |
| B-005 function-shadowed `unset` | resolution order in `CommandCollector::collect_static_tokens` | `force_push_rule_resolves_unset_function_before_builtin_state` covers `f(){ git push --force; }; unset(){ :; }; unset -f f; f` → Block |
| B-006 explicit builtin and function ordering | shared builtin command-position normalization and function-aware shell-state/wrapper mutation | focused tests cover `builtin unset -f f`, `builtin command unset -f f`, and `builtin -- unset -f f` → Allow, function-shadowed positional `trap`/`env`/`alias` calls, and their ordinary non-shadowed builtin or wrapper behavior |
| B-007 bounded deterministic behavior | existing parser/expansion limits, `MAX_STATIC_WORD_VARIANTS`, critical positional prioritization, and evaluator regression suite | bounded positional regression retains a critical 301st candidate without unbounded state; `cargo test -q rules::evaluator --lib` passes with no new external calls or mutable global state |
| B-008 paired bypass/precision evidence | `src/rules/evaluator/tests/git_execution.rs`, `git_execution_wrapper_options.rs`, `git_execution_positional_regressions.rs`, and `git_execution_alternative_state.rs` | focused block/allow tables pass; quoted defaults, parent-expanded heredocs, nested single-quoted substitution, top-level assignment status, readonly function-versus-variable state, correlated setup/state alternatives, and brace-expanded non-shell argv remain precise while adjacent executable forms Block |

## Data Flow

```text
static Bash command text
  -> Brush AST and bounded token variants
  -> command-position / shell-basename normalization
  -> function resolution OR explicit builtin state mutation
  -> shell -c positional expansion when statically known
  -> recursive child-shell parsing
  -> structural Git force-push predicate
  -> Allow | Warn | Block
```

There are no database writes, network calls, migrations, or new persisted
fields. Release metadata changes only stage the required source version.

## Alternatives Considered

- Add special-case string replacements in the Git predicate: rejected because
  shell semantics belong in the AST normalization layer and must also apply to
  nested commands.
- Spawn Bash to expand `-c` arguments: rejected because hook evaluation must be
  deterministic, local, bounded, and free of code execution.
- Treat every `unset -f` token sequence as builtin syntax: rejected because it
  reproduces the false block when a function shadows `unset`.
- Recognize arbitrary case-insensitive `.EXE` names: rejected because the issue
  evidence establishes the exact `.exe` form and broader Windows command-name
  normalization is not specified.

## Risks

- Security: Under-expansion can miss a forbidden force push; over-expansion can
  false-block. Paired fixtures cover both directions, and dynamic values remain
  conservative.
- Compatibility: Existing suffix-free shell and function-call semantics must
  remain byte-for-byte equivalent at the evaluator output boundary.
- Performance: The design reuses bounded in-memory expansion and adds no I/O;
  the full evaluator suite guards against accidental recursion changes.
- Maintenance: Function and shell positional semantics share mechanics but
  retain explicit `$0` mappings to avoid future index drift.

## Test Plan

- [x] Focused red/green fixtures for `.exe` recognition, shell `-c` positional
      binding, and shadowed-versus-explicit-builtin `unset`.
- [x] `cargo test -q rules::evaluator --lib`
- [x] `python3 checks/check_workflow.py --repo . --spec-dir specs/GH860`
- [x] `python3 scripts/ci/check_plugin_version_sync.py`
- [x] `python3 scripts/ci/check_version_bump.py origin/main WORKTREE`
- [x] `cargo fmt --check`
- [x] `cargo check`
- [ ] `cargo test`
- [x] `cargo clippy --all-targets -- -D warnings`

The focused evaluator suite passes 126 tests on the current implementation.
The fresh worktree-local full suite reports 2702 passed, 1 ignored, and the
single unrelated path-sensitive `writer_refuses_high_context_paths` failure
caused by this required worktree living below `.codex/`. Hosted CI in a normal
checkout remains required before merge; this document does not present that
local path-classification mismatch as a passing full-suite result.

## Rollback Plan

Revert the evaluator, fixtures, contract delta, and synchronized `0.6.9`
staging together before release. No stored data or artifact schema changes need
rollback. Do not leave a lower Cargo version paired with higher plugin/npm/MCP
metadata, and do not publish or fabricate release assets as part of rollback.

<!-- specrail-planned-changes
{
  "version": 1,
  "issue": 860,
  "complete": true,
  "paths": [
    "specs/GH860/product.md",
    "specs/GH860/tech.md",
    "specs/GH860/tasks.md",
    "src/rules/evaluator.rs",
    "src/rules/evaluator/bash_ast.rs",
    "src/rules/evaluator/bash_ast/alternative_state.rs",
    "src/rules/evaluator/bash_ast/command_resolution.rs",
    "src/rules/evaluator/bash_ast/control_flow.rs",
    "src/rules/evaluator/bash_ast/shell_state.rs",
    "src/rules/evaluator/bash_ast/static_execution.rs",
    "src/rules/evaluator/bash_ast/function_args.rs",
    "src/rules/evaluator/bash_ast/function_args/parameters.rs",
    "src/rules/evaluator/bash_ast/function_args/quoting.rs",
    "src/rules/evaluator/bash_ast/static_words.rs",
    "src/rules/evaluator/bash_ast/stdin_payload.rs",
    "src/rules/evaluator/bash_ast/unwrap.rs",
    "src/rules/evaluator/tests.rs",
    "src/rules/evaluator/tests/git_execution.rs",
    "src/rules/evaluator/tests/git_execution_alternative_state.rs",
    "src/rules/evaluator/tests/git_execution_positional_regressions.rs",
    "src/rules/evaluator/tests/git_execution_wrapper_options.rs",
    "docs/specs/preference-rule-compilation/TECH.md",
    "Cargo.toml",
    "Cargo.lock",
    "plugins/remem/.codex-plugin/plugin.json",
    "plugins/remem/runtimes/remem-releases.json",
    "npm/remem/package.json",
    "server.json",
    "CHANGELOG.md"
  ],
  "spec_refs": [
    "specs/GH860/product.md",
    "specs/GH860/tech.md",
    "docs/specs/preference-rule-compilation/TECH.md"
  ]
}
-->

This document does not claim `spec_approval`. The current `implx auto`
invocation authorizes drafting and implementation for this bounded issue, while
the normal independent review, CI, PR-gate, and merge-evidence requirements
remain in force.
