# Public Benchmark Artifacts

This directory contains checked-in public benchmark artifact layouts for
`remem bench verify`. The smoke files validate schema shape, relative paths,
required logs, temporary `REMEM_DATA_DIR` evidence, and private-path guards
without invoking an agent or external model.

`memory/suites/remem-code-memory/` is the first remem-native memory capability
suite. It tests coding-memory QA behavior without asking an agent to edit code.
The committed `remem_default` report covers temporal/as-of questions, stale
decision avoidance, conflict detection, workstream continuity, prior bug root
cause, architecture constraints, file/source anchors, and user-context
relevance.

Run artifact verification:

```bash
cargo run -- bench verify --root eval/public --json-out /tmp/remem-bench-verify.json
```

Regenerate the committed memory-suite report and artifacts:

```bash
cargo run -- bench memory --suite remem-code-memory --condition remem_default --root eval/public --artifact-prefix memory/artifacts/remem-code-memory-v1 --json-out eval/public/memory/reports/remem-code-memory-v1.json
```

Invalid examples under `invalid-examples/` are not discovered by the verifier
because they are not under a `manifests/` directory. Unit tests use equivalent
fixtures to prove negative cases.
