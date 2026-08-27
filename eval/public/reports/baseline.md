# remem Public Baseline Directional Report

Claim level: `directional_only_no_public_claim`.

This report separates memory-system capability evidence from coding-agent outcome evidence. It is directional only and does not support SOTA, broad superiority, or coding-task superiority claims.

## Artifact Verification

- Passed: `true`
- Manifests checked: `6`
- Reports checked: `6`
- Run artifacts checked: `65`
- Artifact files checked: `365`

## Memory-System Capability

| Report | Runs | Claim level | Answer score | Support coverage | Citation recall | Non-retention leak rate |
|---|---:|---|---:|---:|---:|---:|
| `adversarial-policy` | 15 | `directional_memory_suite_no_public_claim` | 1.000 | 0.067 | 0.067 | 0.000 |
| `adversarial-policy` | 20 | `directional_memory_suite_no_public_claim` | 0.950 | 0.050 | 0.050 | 0.000 |
| `adversarial-policy` | 20 | `directional_memory_suite_no_public_claim` | 0.950 | 0.050 | 0.050 | 0.000 |
| `remem-code-memory-smoke` | 1 | `smoke_only_no_public_claim` | n/a | n/a | n/a | n/a |
| `remem-code-memory` | 8 | `directional_memory_suite_no_public_claim` | 1.000 | 1.000 | 1.000 | n/a |

## Coding-Agent Outcome

| Condition | Runs | Resolved rate | Token mean | Token variance | Wall-time mean ms | Variance status |
|---|---:|---:|---:|---:|---:|---|
| `remem_preloaded` | 1 | 1.000 | 125.000 | n/a | 1000.000 | `insufficient_runs_for_variance` |

## Paired Coding Statistics

| Comparison | Report | Status | Treatment rate | Control rate | Effect pp | 95% CI pp | Method | Reason |
|---|---|---|---:|---:|---:|---:|---|---|
| `remem-e2e-vs-no-memory-v1` | `n/a` | `insufficient` | n/a | n/a | n/a | n/a | `task_cluster_paired_bootstrap_v1` | requires one verified issue385-v1/official-v1 report containing no_memory, remem_e2e, and curated_file_budgeted for all 16 registered tasks with run indices 0, 1, and 2 |
| `remem-e2e-vs-curated-file-budgeted-v1` | `n/a` | `insufficient` | n/a | n/a | n/a | n/a | `task_cluster_paired_bootstrap_v1` | requires one verified issue385-v1/official-v1 report containing no_memory, remem_e2e, and curated_file_budgeted for all 16 registered tasks with run indices 0, 1, and 2 |

## Coding Task Outcomes

| Task | Condition | Run | Resolved | Failure reason | Tokens | Wall time ms | Memory helped | Memory hurt |
|---|---|---:|---|---|---:|---:|---|---|
| `smoke-fix-startup-race-001` | `remem_preloaded` | 0 | `true` | `none` | 125 | 1000 | `true` | `false` |

## Failure Decomposition

Coding failure counts:

- none

Coding memory-specific failure counts:

- none

Memory gap counts:

- `policy_abstention`: 50
- `retrieval_side_gap`: 16
- `write_side_gap`: 36

## Reproducibility

Run these commands from a clean checkout:

```bash
cargo run -- bench verify --root eval/public --json-out /tmp/remem-public-bench-verify.json
cargo run -- bench report --root eval/public --json-out eval/public/reports/baseline.json --markdown-out eval/public/reports/baseline.md
cargo run -- bench coding --suite issue385-v1 --dry-run --json-out /tmp/remem-issue385-v1-dry-run.json
cargo run -- bench memory --suite remem-code-memory --condition remem_default --root eval/public --artifact-prefix memory/artifacts/remem-code-memory-v1 --json-out eval/public/memory/reports/remem-code-memory-v1.json
cargo run -- bench memory --suite adversarial-policy --condition remem_default --root eval/public --artifact-prefix memory/artifacts/adversarial-policy-v1 --json-out eval/public/memory/reports/adversarial-policy-v1.json
```

Locks and evidence are recorded in the JSON report under `reproducibility`, including remem commits, fixture revisions, Docker image digests, prompt hashes, model labels, and repo base commits when present.

## Claim Gate

- Artifact verifier passed: `true`
- Coding outcome stop-loss status: `not_evaluated_insufficient_coding_matrix`
- Public SOTA status: `not_evaluated_no_public_sota_claim`
- This baseline is directional only and must not be used for coding-task superiority claims.
- README and release wording must not claim SOTA or coding outcome improvement from this report.
- Coding artifacts are not the registered issue385-v1/official-v1 official matrix.
