# Tech Spec

## Linked Issue

GH-863

## Product Spec

[`product.md`](product.md)

## Codebase Context

| Area | Files | Current behavior | Why relevant |
| --- | --- | --- | --- |
| Sync import guard | `scripts/sync-specrail-checks.sh` | The embedded Python verifier parses every classified `checks/*.py` file, classifies static and selected literal dynamic imports, then imports each classified module. It does not reject importlib loader namespaces or direct built-in `exec`/`eval`. | Both GH-863 bypass classes occur between AST parsing and module execution in this verifier. |
| Isolated guard fixtures | `scripts/ci/test_specrail_gate_wiring.py` | A temporary copied pack proves unclassified imports, aliases, `sys.path` mutation, path escape, symlinks, and sourceless modules fail before helper side effects execute. | GH-863 needs regression cases in the same pre-execution sentinel harness. |
| SpecRail packet | `specs/GH863/product.md`, `specs/GH863/tech.md`, `specs/GH863/tasks.md` | No GH-863 packet existed before this tranche. | The implementation route requires complete issue-local spec coverage. |

## Proposed Design

Extend the existing AST preflight in `verify_python_imports` with two
conservative, syntax-directed rejection policies:

1. Importlib submodule allowlist:
   - keep the existing accepted forms for bare `import importlib` followed by a
     direct literal `importlib.import_module(...)`, and
     `from importlib import import_module`;
   - reject `import importlib.<submodule>`;
   - reject `from importlib import <name>` when `<name>` is not
     `import_module`; and
   - reject `from importlib.<submodule> import ...`.
   - reject literal dynamic-import targets `importlib`, `importlib.*`, and
     the frozen importlib implementation modules, as well as `sys`; and
   - reject direct or named access to loader-bearing `sys` namespaces;
   - reject `__loader__`, `__spec__`, and frozen importlib implementation
     modules; and
   - reject `globals`/`locals`/`vars` namespace access that can recover those
     loader surfaces indirectly.

   The rejection is intentionally broader than enumerating current loader
   classes. It prevents a new loader-construction API from silently reopening
   the bypass and gives maintainers one explicit allowlist boundary.

2. Dynamic code-execution surface rejection:
   - collect aliases introduced by `from builtins import exec|eval`;
   - reject any `ast.Name` reference to direct `exec`/`eval` or those aliases;
   - reject `builtins.exec`/`builtins.eval` attribute references for known
     builtins module aliases; and
   - reject non-`__import__` named imports from `builtins`, literal dynamic
     imports of `builtins`, and direct `__builtins__` namespace access; and
   - reject dynamic `globals`/`locals`/`vars` namespace access that can recover
     `__builtins__` indirectly; and
   - run this rejection before the existing dynamic-import alias analysis and
     before any classified module import.

The verifier will emit stable prefixes:

- `UNSUPPORTED IMPORTLIB LOADER SURFACE`
- `UNSUPPORTED DYNAMIC CODE EXECUTION`

Each message includes the classified source path and the rejected symbol or
module. The policy is deliberately reference-based rather than call/data-flow
based: assignment or storage of one of these callables is rejected too, and
literal `exec`/`eval` is rejected along with file-derived content.

No lock schema, synchronized-file list, runtime database, or installed hook
behavior changes.

## Product-to-Test Mapping

| Product invariant | Implementation area | Verification |
| --- | --- | --- |
| `B-001`, `B-002` | Import and sensitive-module namespace scan in `verify_python_imports` | Temporary-pack fixtures for static and literal-dynamic importlib loaders, loader-bearing `sys` namespaces, module loader metadata, frozen loader modules, and dynamic namespace lookup fail with the loader-surface diagnostic. |
| `B-003` | Builtins import/namespace and AST reference scan | Fixtures for direct `exec`/`eval`, named builtins aliases, dynamically imported builtins, `__builtins__`, builtins dictionaries, and dynamic namespace lookup fail with the code-execution diagnostic. |
| `B-004` | New diagnostic branches | Every new fixture asserts the stable diagnostic and classified source path. |
| `B-005` | Existing sentinel-based isolated harness | Every rejected loader/exec/eval fixture asserts `untrusted-helper-executed` was not created. |
| `B-006`, `B-007` | Existing import and sync verifier behavior | The full `test_specrail_gate_wiring.py` suite and `scripts/sync-specrail-checks.sh --verify` remain green. |
| `B-008` | Existing AST error path plus fail-before-import ordering | Existing syntax/read failure behavior remains unchanged; source analysis completes before the import loop. |

## Data Flow

```text
classified checks/*.py source
  -> ast.parse
  -> collect import/module/callable aliases
  -> reject non-allowlisted and module-provided loader namespaces
  -> reject exec/eval names and dynamic global namespace access
  -> existing static/dynamic import classification
  -> import classified module only if every source passed
```

Inputs remain the sync lock, repository root, tracking mode, and optional
newly-copied managed paths. Output remains process success or a nonzero exit
with diagnostics on stderr. There is no persistence or external call beyond the
existing local Git queries and module imports.

## Alternatives Considered

- Trace only `exec`/`eval` arguments that originate from file reads. Rejected
  because a lightweight AST data-flow analysis would be incomplete around
  aliases and helpers, while the synchronized check set has no current need for
  dynamic code execution.
- Enumerate only `SourceFileLoader`, `spec_from_file_location`, and
  `module_from_spec`. Rejected because other loader classes or future APIs
  would retain the same bypass.
- Sandbox loader and `exec`/`eval` execution. Rejected because executing
  unclassified code to decide whether it is safe violates the pre-execution
  classification boundary.
- Leave the residual risk documented. Superseded for this tranche by the
  current `implx auto` instruction to implement GH-863 fully.

## Risks

- Security: This closes honest-mistake bypasses but is not a security boundary
  against a committer who can modify the verifier. Diagnostics and docs must
  not overstate the threat model.
- Compatibility: A future synchronized check that legitimately needs an
  importlib submodule or `exec`/`eval` will fail closed and require an explicit
  design change rather than silently passing.
- Performance: The added AST scans are linear in the already-parsed syntax
  trees and do not add subprocesses or file reads.
- Maintenance: The allowlist must remain narrow; adding a loader-capable
  importlib name or another implicit module namespace without corresponding
  local-file classification would reopen the issue.

## Test Plan

- [ ] Regression-first: add isolated temporary-pack cases and confirm they fail
      against the current verifier because the bypass executes or reaches the
      wrong diagnostic.
- [ ] Focused integration:
      `python3 scripts/ci/test_specrail_gate_wiring.py`.
- [ ] Sync verification: `scripts/sync-specrail-checks.sh --verify`.
- [ ] Workflow pack:
      `python3 checks/check_workflow.py --repo .`.
- [ ] Issue packet:
      `python3 checks/check_workflow.py --repo . --spec-dir specs/GH863`.
- [ ] Repository pre-completion: `cargo fmt --check` and `cargo check`, run
      only from the GH-863 worktree.
- [ ] Repository submission: `cargo test`, run only from the GH-863 worktree.

## Rollback Plan

Revert the verifier branches and their GH-863 fixtures together. The rollback
requires no data migration, lock rewrite, or cleanup. If a legitimate
synchronized check needs one rejected surface, prefer replacing it with an
ordinary classified import; any future exception should receive a separate
spec and tests that prove local-file classification remains complete.
