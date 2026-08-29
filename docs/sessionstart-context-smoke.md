# SessionStart Context Smoke Matrix

Status: Current verification guide

Run the wrapper from the repository root. It builds the current checkout, reads
the exact executable path from Cargo's `compiler-artifact` output, and passes
that artifact to the fixture. This also works with a configured build target or
`CARGO_TARGET_DIR`.

<!-- remem-doc-contract:isolated-sessionstart-smoke:start -->
```bash
python3 scripts/ci/run_sessionstart_context_gate_smoke.py
```
<!-- remem-doc-contract:isolated-sessionstart-smoke:end -->

The wrapper is the CI and local-preflight entry point. The underlying
[`scripts/ci/smoke_sessionstart_context_gate.sh`](../scripts/ci/smoke_sessionstart_context_gate.sh)
fixture requires exactly one existing, executable, absolute binary path. It
initializes an encrypted store under an isolated temporary `HOME` and
`REMEM_DATA_DIR`, invokes the same Codex SessionStart request twice, and owns
the assertions and cleanup.

Expected checks:

- The first invocation emits non-empty context.
- The second byte-identical, unchanged-session invocation emits exactly zero bytes.
- Initialization or assertion failures exit non-zero with a diagnostic.
- The isolated temporary home, encrypted store, and captured output are removed on exit.
