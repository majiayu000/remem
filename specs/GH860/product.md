# Product Spec

## Linked Issue

GH-860

Complexity: small

## User Problem

The compiled preference-rule evaluator statically recognizes common Bash forms
so it can warn or block an honest coding agent before a forbidden Git force
push runs. Three plausible command forms still produce incorrect results:
Git-for-Windows shell executables ending in `.exe` are not recognized, `bash
-c` positional arguments are not bound into the static script, and a
user-defined function named `unset` is mistaken for the Bash builtin. These
gaps can either miss a forbidden command or report a false block.

## Goals

- Recognize the remaining three common, statically knowable Bash forms from
  GH-860.
- Preserve the evaluator's existing bounded, deterministic, local behavior.
- Keep bypass detection and false-block precision covered together.

## Non-Goals

- Turning the evaluator into a sandbox against deliberately dynamic evasion.
- Executing shell code, consulting a live shell, or interpreting non-static
  values.
- Reopening deferred GH-860 items already covered by the merged GH-671 work.
- Changing rule compilation, rule actions, hook installation, or host support.

## Behavior Invariants

1. B-001 When a statically parsed command invokes a supported shell basename
   with the Windows/Git-for-Windows `.exe` suffix, such as `bash.exe`, the
   evaluator shall apply the same shell-payload analysis as for the equivalent
   suffix-free basename.
2. B-002 Path-qualified supported `.exe` shell commands shall be recognized by
   basename with either POSIX `/` or Windows `\\` separators, independent of
   the remem host OS, while unrelated executables whose names merely contain a
   shell name shall remain ordinary commands.
3. B-003 For a statically known shell `-c` invocation, the evaluator shall bind
   the operands after the command string according to Bash semantics: the
   first operand supplies `$0` and later operands supply positional parameters
   used by the command string. A forbidden force-push argument supplied through
   `$1` shall therefore remain detectable, without replacing positional
   parameters inside a function definition before that function is invoked
   with its own arguments. The mapping shall remain active for deferred EXIT
   traps and for expandable heredoc text before it is handed to a nested shell;
   quote characters inside an unquoted-delimiter heredoc body shall not suppress
   that expansion, while a quoted delimiter shall keep the body literal.
   Unquoted whole-word positionals may yield multiple argv fields; arithmetic
   and nested command-substitution contexts shall parse the expanded source
   under their own Bash syntax rather than inheriting surrounding quote state.
   Empty unquoted expansions shall remove their argv field, quoted default
   words shall retain their grouping, and statically selected `+`/`:+`
   alternative words shall be materialized. Known-set `${n?word}`,
   `${n:?word}`, `${n=word}`, and `${n:=word}` forms shall retain the known
   operand. Static non-negative `${@:offset}` slices and
   `${n:offset[:length]}` substrings shall retain their Bash field or string
   semantics. A definite `set --` or argument-bearing `set -` shall
   replace the active positional mapping; a definite static `shift` shall
   advance it, while possibly executed changes shall retain every possible
   mapping for conservative matching, including when a possible positional is
   concatenated with literal command text. Distinct possible mappings shall be
   evaluated as alternative argv sets rather than flattened into one synthetic
   command. Quoted `"$@"` shall preserve one field per operand. Positional
   changes inside a
   subshell, command substitution, or non-final pipeline process shall not
   leak into its parent, and an alias named `set` shall resolve before builtin
   positional state is applied. Each possible mapping shall retain its own
   command-position argv grouping. Static `${@:offset}` collection slices,
   `${n:offset}` substrings, `shift`, and the argument-bearing `set -` form
   shall update or expand the known mapping with Bash semantics. A function
   named like a stateful builtin, including `trap`, shall resolve before that
   builtin state can be installed; alias resolution shall likewise precede
   `unset -f` and function-export state. A failed static `shift` shall expose a
   failing status to `&&`/`||` reachability, and `${@:0}` shall include the
   known `$0`. Expandable outer heredocs shall finish
   parent-side expansion before entering a child `-c` scope, while explicit
   arguments to `source [--] /dev/stdin` shall bind `$1...` for the sourced
   body, then restore caller positionals unless the sourced body performs a
   definite `set --`; that mutation shall persist for subsequent control-flow
   branches as it does in Bash. Static shells reading stdin, including
   `bash -s -- args`, shall
   bind their post-option operands as `$1...`. Collection default/alternative
   forms such as `${@:-word}` and `${@:+word}` shall select statically when the
   operand set is known, and recognized `set` options before `--` shall not
   hide the positional assignment that follows. A command name materialized
   from a positional shall not be
   reclassified as an assignment or passed through lexical alias expansion,
   while recognized wrapper semantics remain active; a here-string positional
   shall preserve embedded source newlines. When statically known paths
   diverge, each command, alias, function, builtin fallback, and statically
   fallible readonly-assignment or redirection setup outcome shall execute
   against an isolated full shell-state snapshot. An ordinary assignment
   prefix shall not invent a setup-failure path. Setup failure shall preserve
   pre-command state and a failing status. An ordinary assignment prefix shall
   preserve the known status of `true`/`false`/`:` both inside and outside a
   child shell positional context. Function-mode `readonly -f` operands shall
   not be recorded as readonly variable names, while `readonly -p NAME` shall
   retain Bash's variable declaration behavior. Normalized `command`/`builtin`
   wrappers, including `builtin --`, shall retain known
   `true`/`false`/`:` status; every terminating alternative shall run its EXIT
   traps before it can be filtered; and state from a terminated path shall not
   contaminate a continuing path.
4. B-004 Missing shell `-c` operands and positional references without a known
   operand shall remain unresolved and shall not be invented, shifted, or
   borrowed from surrounding commands.
5. B-005 When static shell state contains a function named `unset`, invoking
   `unset -f <name>` shall be analyzed as that function call and shall not
   remove `<name>` from the evaluator's function state merely because its argv
   resembles the builtin.
6. B-006 An explicit Bash builtin invocation, such as `builtin unset -f
   <name>`, shall retain builtin unset semantics even when a function named
   `unset` exists, including valid mixed `builtin command` wrapper sequences.
   The same function-before-builtin ordering applies to static EXIT-trap state:
   functions named `trap`, `env`, or `alias` shall run as functions before
   builtin-like trap capture, `env -S` splitting, or alias-state mutation,
   while the corresponding non-shadowed commands remain analyzable.
7. B-007 Each new recognition path shall be bounded and deterministic. Possible
   positional mappings shall share the existing 256-variant ceiling and retain
   security-relevant mappings first; dynamic or unsupported shell constructs
   shall preserve the existing conservative behavior.
8. B-008 Regression fixtures shall cover both the newly detected force-push
   forms and nearby allowed forms so the precision fixes do not weaken existing
   bypass coverage or create false blocks.

## Acceptance Criteria

- [x] A red-first fixture proves `bash.exe` and a path-qualified supported
      `.exe` shell detect a static forbidden Git force push, with a nearby
      unrelated `.exe` command remaining allowed.
- [x] A red-first fixture proves `bash -c 'git push "$1"' _ --force` is
      detected and a missing/unrelated positional value does not fabricate a
      match.
- [x] Red-first fixtures cover zero-field positional removal, quoted default
      grouping, `+`/`:+` alternatives, `set --`, parent-expanded heredoc stdin,
      explicit `source /dev/stdin` arguments, command-word provenance, and
      here-string source text.
- [x] Red-first fixtures cover quoted `"$@"` cardinality, uncertain `set --`
      alternatives, child-scope restoration, and alias-before-builtin ordering.
- [x] Red-first fixtures cover possible command-position grouping, positional
      slices and substrings, definite `shift`, argument-bearing `set -`, and
      function-shadowed `trap` ordering.
- [x] Focused fixtures cover uncertain concatenated positionals and prove that
      positional `env`/`alias` command names still honor function lookup before
      wrapper splitting or builtin-like state mutation.
- [x] Focused fixtures prove possible multi-field argv mappings remain separate
      and the bounded mapping set retains a late security-critical alternative.
- [x] Red-first fixtures cover mutually exclusive last-option-wins force flags,
      `${@:0}`, failed-`shift` control flow, and alias-before-`unset -f` state.
- [x] Red-first fixtures cover correlated path-specific `set` transitions,
      mixed-success `shift` branches, and all-path alias/function presence
      across uncertain redefinitions.
- [x] Red-first fixtures cover isolated full shell-state alternatives,
      possible function/builtin fallback, readonly-assignment/redirection
      setup, ordinary assignment-prefix status in child and top-level scopes,
      readonly function-versus-variable state, wrapper-normalized status,
      terminating-path filtering, and EXIT trap collection for every terminated
      alternative.
- [x] Focused fixtures cover `source -- /dev/stdin`, temporary source operands,
      caller-positional restoration, and possible positional heredoc expansion.
- [x] A red-first fixture proves a function-shadowed `unset -f` does not erase
      the target function, while `builtin unset -f` still does.
- [x] Existing rule-evaluator tests continue to pass.
- [ ] Repository formatting, build, workflow, and full test gates pass.

## Edge Cases

- Supported shell names without `.exe` keep their current behavior.
- Static path qualification changes only basename recognition, not path
  resolution or filesystem access.
- Quoted and concatenated positional references follow the evaluator's
  existing static word-expansion limits; unknown expansion remains unknown.
- Definite static `set --` and explicit sourced-file arguments follow their
  Bash-defined positional scope, including persistence of a sourced body's
  definite `set --`; uncertain `set --` retains every known
  possible mapping so a forbidden argument on either path remains visible.
  Later state changes transform each mapping once and retain its status/path
  correlation through an immediate `&&` or `||` branch.
- Function shadowing is evaluated in command order and existing subshell,
  pipeline, and child-shell scope rules remain unchanged.

## Boundary Checklist

| Boundary | Verdict |
| --- | --- |
| Empty / missing input | Covered by B-004. |
| Error and failure paths | Covered by B-004 and B-007; unresolved static values stay conservative. |
| Authorization / permission | N/A: this is a pure local evaluator with no authorization state. |
| Concurrency / race / ordering | Covered by B-005 and the command-order requirement in Edge Cases; concurrency is N/A for pure evaluation. |
| Retry / repetition / idempotency | Covered by B-007; identical input produces identical output. |
| Illegal state transitions | Covered by B-005 and B-006 for function-state mutation. |
| Compatibility / migration | Covered by B-001, B-002, and B-007; no stored-data migration exists. |
| Degradation / fallback | Covered by B-004 and B-007; unknown values are not presented as successful static resolution. |
| Evidence and audit integrity | Covered by B-008 through paired positive and negative fixtures. |
| Cancellation / interruption / partial completion | N/A: evaluation is an in-process bounded pure operation. |

## Rollout Notes

No configuration or migration is required. The change ships as a compatible
precision update to the existing artifact-v2 structural force-push evaluator.
