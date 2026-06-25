# Public Benchmark Artifact Smoke Fixture

This directory contains the first checked-in public benchmark artifact layout
for `remem bench verify`. The files are smoke fixtures only: they validate
schema shape, relative paths, required logs, temporary `REMEM_DATA_DIR`
evidence, and private-path guards without invoking an agent or external model.

The real memory suites, coding task pack, baselines, and report generator land
in follow-up issues.

Run:

```bash
cargo run -- bench verify --root eval/public --json-out /tmp/remem-bench-verify.json
```

Invalid examples under `invalid-examples/` are not discovered by the verifier
because they are not under a `manifests/` directory. Unit tests use equivalent
fixtures to prove negative cases.
