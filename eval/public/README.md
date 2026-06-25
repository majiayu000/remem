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

The memory runner also supports diagnostic and baseline conditions for
write-vs-retrieval analysis: `truncated_full_context`, `oracle_evidence`,
`complete_stored_memory`, `retrieved_memory`, `bm25_baseline`,
`vector_baseline`, `hybrid_rag_baseline`, and `summary_baseline`. Generated
reports include `failure_decomposition` and per-condition latency/token
metrics so misses can be attributed to write-side evidence loss, retrieval
miss, reader failure, or policy abstention.

`memory/suites/adversarial-policy/` covers memory non-retention policy behavior.
The committed `remem_default` report checks that secrets, credentials, payment
data, unframed third-party personal details, roleplay, negations, unsupported
assistant claims, unapproved external sources, spliced claims, same-name repos,
multi-task bleed, branch divergence, stale file anchors, and unresolved
conflicts do not leak into active memory outputs unless explicitly approved.

`../coding-bench/fixtures/tasks.json` is the public `issue385-v1` coding-agent
task pack. It contains 16 deterministic tasks across the required memory
dependency categories plus a smoke subset:

Coding-agent artifacts use a fixed `failure_reason` enum and, for `remem`
runs, a `memory_contract` block that records injected memory ids, cited/used
memory ids, citation precision/recall, stale used count, irrelevant injection
count, missing relevant memory count, `memory_helped`, and `memory_hurt`.

```bash
cargo run -- bench coding --suite issue385-v1 --dry-run --json-out /tmp/remem-issue385-v1-dry-run.json
cargo run -- bench coding --suite issue385-v1 --task-set smoke --dry-run --json-out /tmp/remem-issue385-v1-smoke-dry-run.json
```

Run artifact verification:

```bash
cargo run -- bench verify --root eval/public --json-out /tmp/remem-bench-verify.json
```

Generate the checked-in directional baseline report:

```bash
cargo run -- bench report --root eval/public --json-out eval/public/reports/baseline.json --markdown-out eval/public/reports/baseline.md
```

The baseline report separates memory-system capability evidence from
coding-agent outcome evidence. It remains `directional_only_no_public_claim`
until the coding-agent artifacts include `no_memory`, `remem`, and
`curated_file` with at least three runs per condition and the public claim gate
passes.

Regenerate the committed memory-suite report and artifacts:

```bash
cargo run -- bench memory --suite remem-code-memory --condition remem_default --root eval/public --artifact-prefix memory/artifacts/remem-code-memory-v1 --json-out eval/public/memory/reports/remem-code-memory-v1.json
```

Generate a full diagnostic memory-suite report without replacing committed
artifacts:

```bash
cargo run -- bench memory --suite remem-code-memory --json-out /tmp/remem-memory-diagnostics.json
```

Regenerate the committed adversarial-policy report and artifacts:

```bash
cargo run -- bench memory --suite adversarial-policy --condition remem_default --root eval/public --artifact-prefix memory/artifacts/adversarial-policy-v1 --json-out eval/public/memory/reports/adversarial-policy-v1.json
```

Invalid examples under `invalid-examples/` are not discovered by the verifier
because they are not under a `manifests/` directory. Unit tests use equivalent
fixtures to prove negative cases.
