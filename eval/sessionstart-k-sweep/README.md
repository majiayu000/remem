# SessionStart relevance k sweep

This report implements the approved GH-854 four-arm decision. It reuses the
existing golden evaluator and eval-gates; it does not introduce a second gate.

Run from the repository worktree:

```bash
for k in 1 3 5 10; do
  cargo run --quiet -- eval --dataset eval/golden.json --json -k "$k"
  REMEM_CONTEXT_RELEVANCE_K="$k" \
    cargo run --quiet -- eval-gates --json-out "/tmp/remem-eval-gates-k${k}.json"
done
```

Every populated slice has the same `hit_at_k` in all four arms, and every
eval-gates arm passes with zero reported capacity degradation. The approved
smallest-eligible rule therefore selects `k=1`.

The secondary tradeoff is explicit: compared with k=3/5/10, k=1 improves
overall precision from 0.477778 to 0.516667 while reducing overall recall and
evidence recall from 0.516667 to 0.433333. Set
`REMEM_CONTEXT_RELEVANCE_K=0` for the legacy governed-section rollback.

See [`report.json`](report.json) for source revision, dataset and artifact
hashes, every populated slice, output characters, capacity results, and the
decision record.
