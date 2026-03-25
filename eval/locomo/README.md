# LoCoMo Benchmark for remem

Run LoCoMo (Long Conversation Memory) benchmark against remem's REST API.

## Quick Start

```bash
# 1. Download dataset
python download_data.py

# 2. Start remem API server
remem api --port 7899

# 3. Run benchmark (ingest + retrieve + answer + judge)
python run_locomo.py --remem-url http://127.0.0.1:7899 --model claude-3-5-haiku-20241022

# 4. Generate scores report
python score.py --results results/locomo_results.json
```

## Cost Estimate

- ~1986 QA pairs (excluding category 5 adversarial)
- Answer generation: ~1986 x ~800 tokens = ~1.6M tokens (~$0.80 with Haiku)
- LLM judge: ~1986 x ~300 tokens = ~0.6M tokens (~$0.05 with GPT-4o-mini)
- Total: ~$0.85 per full run
