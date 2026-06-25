# remem Public Baseline Directional Report

Claim level: `directional_only_no_public_claim`.

This report separates memory-system capability evidence from coding-agent outcome evidence. It is directional only and does not support SOTA, broad superiority, or coding-task superiority claims.

## Artifact Verification

- Passed: `true`
- Manifests checked: `4`
- Reports checked: `4`
- Run artifacts checked: `25`
- Artifact files checked: `125`

## Memory-System Capability

| Report | Runs | Claim level | Answer score | Support coverage | Citation recall | Non-retention leak rate |
|---|---:|---|---:|---:|---:|---:|
| `adversarial-policy` | 15 | `directional_memory_suite_no_public_claim` | 1.000 | 0.067 | 0.067 | 0.000 |
| `remem-code-memory-smoke` | 1 | `smoke_only_no_public_claim` | n/a | n/a | n/a | n/a |
| `remem-code-memory` | 8 | `directional_memory_suite_no_public_claim` | 1.000 | 1.000 | 1.000 | n/a |

## Coding-Agent Outcome

| Condition | Runs | Resolved rate | Token mean | Token variance | Wall-time mean ms | Variance status |
|---|---:|---:|---:|---:|---:|---|
| `remem` | 1 | 1.000 | 125.000 | n/a | 1000.000 | `insufficient_runs_for_variance` |

## Coding Task Outcomes

| Task | Condition | Run | Resolved | Failure reason | Tokens | Wall time ms | Memory helped | Memory hurt |
|---|---|---:|---|---|---:|---:|---|---|
| `smoke-fix-startup-race-001` | `remem` | 0 | `true` | `none` | 125 | 1000 | `true` | `false` |

## Failure Decomposition

Coding failure counts:

- none

Coding memory-specific failure counts:

- none

Memory gap counts:

- `policy_abstention`: 14
- `retrieval_side_gap`: 14

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
- Coding artifacts do not yet include no_memory, remem, and curated_file conditions.
- Coding artifacts do not yet have at least three runs per condition.
