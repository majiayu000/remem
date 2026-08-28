# SessionStart Context Smoke Matrix

Status: Current verification guide

The executable fixture builds the current checkout once, initializes an
encrypted store under an isolated temporary `HOME` and `REMEM_DATA_DIR`, and
invokes the same Codex SessionStart request twice. Run it from the repository
root:

<!-- remem-doc-contract:isolated-sessionstart-smoke:start -->
```bash
scripts/ci/smoke_sessionstart_context_gate.sh
```
<!-- remem-doc-contract:isolated-sessionstart-smoke:end -->

The fixture owns setup, assertions, and cleanup. Its implementation is the
single source of truth; CI and local preflight execute the same entry point.

Expected checks:

- The first invocation emits non-empty context.
- The second byte-identical, unchanged-session invocation emits exactly zero bytes.
- Initialization or assertion failures exit non-zero with a diagnostic.
- The isolated temporary home, encrypted store, and captured output are removed on exit.
