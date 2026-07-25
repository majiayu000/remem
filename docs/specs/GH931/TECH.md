# GH931 Flagship E2E Public Proof — Tech Spec

Status: Current contract (harness scaffold)
Issue: #931

## Harness artifacts (this PR)

- `eval/coding-bench/conditions.json`: machine-readable condition registry —
  3 primary + 7 diagnostic conditions, per-condition `runner_status`
  (`implemented`, `implemented_as_legacy_id`, `artifact_schema_only`,
  `pending_src_support`), isolation rules, `remem_e2e` forbidden shortcuts,
  and the 6-stage / 12-enum failure taxonomy.
- `eval/coding-bench/schemas/conditions.schema.json`: schema for the registry.
- `eval/coding-bench/schemas/curator-log.schema.json` +
  `eval/coding-bench/examples/curator-log.example.json`: the
  `curated_file_budgeted` run artifact contract.
- `eval/coding-bench/curated-file-budgeted-protocol.md`: curator protocol
  (per-session time budget, character cap, edit accounting, freeze hash).
- `eval/coding-bench/validate_schemas.py`: offline validator — schema checks
  plus cross-field rules (budget limits, totals consistency, taxonomy
  completeness, no legacy `remem`/`curated_file` ids, file references exist).
- `eval/claims/registry.json`, `eval/claims/claims-registry.schema.json`,
  `eval/claims/claim_gate.py`: claim registry with pre-registered #931
  thresholds and the wording gate (verdict enum, forbidden-phrase scan,
  `Directional evidence:` prefix for `INSUFFICIENT`, supporting-report SHA-256
  verification). `claim_gate.py --self-test` runs the embedded unit tests.

All validators are Python 3 stdlib only and run offline. The JSON-schema
subset validator lives in `claim_gate.py`; `validate_schemas.py` imports it
instead of duplicating it.

## remem_e2e execution contract (src follow-up, not this PR)

The Rust runner (`src/eval/coding_bench`) needs:

1. `BenchCondition` id rename `remem` → `remem_preloaded` and `curated_file`
   → `curated_file_expert`, no compatibility aliases.
2. New `remem_e2e` condition: feed fixture history episodes through real
   capture (`captured_events`) → extraction_tasks → observations/candidates →
   promotion policy → memories, then serve the target run via the production
   SessionStart/MCP retrieval path only. Hard-fail if the run plan attempts
   direct memory seeding or gold-evidence preloading.
3. `remem_e2e` requires a configured remem LLM provider key at execution time;
   `--dry-run` must not require it.
4. `curated_file_budgeted` run support: inject a curator-produced `MEMORY.md`,
   verify its SHA-256 against the curator log artifact, and attach the log to
   the run artifact.
5. Report additions: failure `stage` attribution (6-stage enum), curator
   maintenance metrics, and paired task-cluster bootstrap statistics.

## Validation commands

```bash
python3 eval/coding-bench/validate_schemas.py
python3 eval/claims/claim_gate.py check
python3 eval/claims/claim_gate.py --self-test
cargo run -- bench coding --suite issue385-v1 --dry-run --json-out /tmp/dry.json
```
