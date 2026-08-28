# SessionStart Context Smoke Matrix

Status: Current verification guide

Build the current checkout, then pass its existing executable to the fixture.
The fixture initializes an encrypted store under an isolated temporary `HOME`
and `REMEM_DATA_DIR`, and invokes the same Codex SessionStart request twice.
Run it from the repository root (use the corresponding absolute path when
`CARGO_TARGET_DIR` is set):

<!-- remem-doc-contract:isolated-sessionstart-smoke:start -->
```bash
cargo build --locked --bin remem
scripts/ci/smoke_sessionstart_context_gate.sh "$(pwd -P)/target/debug/remem"
```
<!-- remem-doc-contract:isolated-sessionstart-smoke:end -->

The fixture requires exactly one existing, executable, absolute binary path.
It owns setup, assertions, and cleanup. Its implementation is the single
source of truth; CI and local preflight build first and then execute the same
entry point.

Expected checks:

- The first invocation emits non-empty context.
- The second byte-identical, unchanged-session invocation emits exactly zero bytes.
- Initialization or assertion failures exit non-zero with a diagnostic.
- The isolated temporary home, encrypted store, and captured output are removed on exit.
