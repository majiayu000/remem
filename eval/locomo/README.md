# LoCoMo Benchmark for remem

Run LoCoMo (Long Conversation Memory) benchmark against remem's REST API.

LoCoMo is informational-only in remem. Do not use it as a CI or release gate;
the checked-in score files are a historical footnote for manual comparison.

## Quick Start

```bash
# 1. Download dataset
python download_data.py

# 2. Start remem API server
remem api --port 7899

# 3. Run benchmark (ingest + retrieve + answer + judge)
OPENAI_API_KEY=... python run_locomo.py --remem-url http://127.0.0.1:7899 --model gpt-5.4
```

`run_locomo.py` writes both raw QA rows and score summaries under
`eval/locomo/results/`. Use `--sample-index N` for one conversation and
`--skip-ingest` when the API database already contains the LoCoMo memories.

## Informational Snapshot

The checked-in score files cover all 10 LoCoMo conversations after adversarial
category skipping:

| Metric | Value |
|---|---:|
| QA pairs | 1540 |
| Correct | 965 |
| Weighted overall | 62.66% |
| Mean sample accuracy | 63.00% |
| Model | gpt-5.4 |

This matches the top-level README `v2 (optimized)` LoCoMo benchmark snapshot,
but it is not a gating target.

## Cost Note

Cost depends on the selected model, answer/judge token lengths, and whether
ingest is re-run. Prefer `--sample-index` first when validating code changes,
then run all 10 conversations once the pipeline is stable.
