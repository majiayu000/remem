# Product Spec

## Linked Issue

GH-863

## User Problem

The SpecRail sync verifier checks that every repository-local Python import used
by synchronized checks is classified in the sync lock before those checks are
executed. Two Python execution paths currently bypass that import-closure
classification:

- importlib loader-construction APIs can load a local file without a normal
  import statement; and
- `exec` or `eval` can execute text read from a local file without entering the
  import graph.

An honest maintainer can therefore add a local helper through one of these
surfaces, update a synchronized file's checksum, and receive a successful sync
verification even though the helper is neither managed nor explicitly
excluded. That weakens the lock's review boundary and can execute the helper
before the verifier reports any problem.

## Goals

- Reject importlib loader-construction surfaces in classified SpecRail Python
  files before any classified check module executes.
- Reject `exec` and `eval` execution surfaces in classified SpecRail Python
  files before any classified check module executes.
- Produce stable, actionable diagnostics naming the affected classified file
  and rejected surface.
- Preserve successful verification for the existing classified static-import
  and literal dynamic-import contract.
- Keep the guard aligned with its honest-mistake threat model.

## Non-Goals

- Defend against a malicious committer who can edit or remove the sync verifier
  itself.
- Build a general-purpose Python taint tracker, sandbox, bytecode analyzer, or
  security scanner.
- Execute a loaded file in a sandbox to decide whether its behavior is safe.
- Add an allowlist for individual loader or `exec`/`eval` call sites in this
  tranche.
- Change which SpecRail files are synchronized, excluded, or represented in
  `checks/specrail-sync.lock.json`.

## Behavior Invariants

1. `B-001`: A classified SpecRail Python file that imports an `importlib`
   submodule or named API outside the existing `import_module` allowlist must
   fail sync verification before any classified check module executes.
2. `B-002`: `B-001` applies to direct submodule imports, named imports, aliases,
   star imports, literal dynamic imports of sensitive importlib namespaces,
   loaded-module namespace access through `sys.modules`, module loader metadata
   such as `__loader__`/`__spec__`, frozen importlib implementation modules,
   and dynamic namespace lookup, including paths that expose `importlib.util`
   and `importlib.machinery`.
3. `B-003`: A classified SpecRail Python file that directly calls or imports,
   aliases, or otherwise references the built-in `exec` or `eval` callable,
   dynamically imports the `builtins` namespace, or reaches the same callables
   through `__builtins__`, an imported builtins dictionary, or dynamic
   `globals`/`locals`/`vars` namespace lookup must fail sync verification before
   any classified check module executes.
4. `B-004`: Rejections must use a stable diagnostic category, include the
   classified source path, and identify whether the rejected surface is an
   importlib loader surface or a dynamic code-execution surface.
5. `B-005`: A rejected file or helper must not produce an observable execution
   side effect during verification.
6. `B-006`: Existing classified static imports and literal
   `importlib.import_module`/`__import__` calls continue to follow their current
   classification behavior for ordinary module targets. Sensitive targets
   `importlib`, `importlib.*`, `builtins`, and `sys` fail closed because they
   expose loader or dynamic-code namespaces outside the ordinary import graph.
7. `B-007`: Existing failures for unclassified local imports, non-literal
   dynamic imports, `sys.path` mutation, path escape, symlinks, sourceless local
   modules, and tracking/lock drift retain their fail-closed behavior.
8. `B-008`: Syntax errors, unreadable classified sources, or a verifier
   analysis failure remain explicit errors; no analysis failure may degrade to
   executing the classified modules.

## Acceptance Criteria

- [ ] Verification rejects direct and aliased imports of
      `importlib.util`/`importlib.machinery` and named loader-construction APIs
      before module execution.
- [ ] Verification rejects the same loader namespaces reached through literal
      dynamic imports, `sys.modules`, loader metadata, frozen importlib modules,
      or dynamic namespace lookup.
- [ ] Verification rejects direct `exec`/`eval`, `builtins.exec`/`eval`, and
      named or aliased builtins imports before module execution.
- [ ] Verification rejects dynamically imported builtins plus
      `__builtins__`/builtins-dictionary/dynamic-namespace access before module
      execution.
- [ ] Regression fixtures prove rejected helpers cannot create their sentinel
      side-effect file.
- [ ] Diagnostics name the classified source and use stable loader-surface or
      dynamic-code-execution categories.
- [ ] The isolated SpecRail sync verifier baseline and all existing import
      classification fixtures continue to pass.
- [ ] `scripts/sync-specrail-checks.sh --verify` and the repository workflow
      checks pass on the final tree.

## Edge Cases

- An importlib submodule is imported under an alias but never called.
- A loader callable or `exec`/`eval` is imported under a different local name.
- `exec`/`eval` is referenced without an immediate call, such as assignment to
  another variable.
- A rejected file also contains an otherwise classified import.
- The text passed to `exec`/`eval` is a literal rather than file content.
  This tranche rejects the execution surface conservatively instead of trying
  to infer data flow.
- The repository is offline. The verification is local and does not require
  network access.

## Rollout Notes

This is a fail-closed CI guard tightening. Existing synchronized checks that use
an importlib submodule or `exec`/`eval` must be rewritten to use ordinary
classified imports rather than granted an implicit exception. No runtime data
migration or user configuration change is required.
