# Golden Query Eval

`remem eval` runs a deterministic retrieval-quality check against a versioned JSON fixture.
This golden eval is the deterministic retrieval gate; LoCoMo remains
informational-only and must not be used as a CI gate.

```bash
remem eval --dataset eval/golden.json -k 5
```

The command reports per-query status plus overall, per-slice, and per-category metrics:

- `H@k`: at least one expected memory or evidence ref appears in the top `k`.
- `MRR@10`: reciprocal rank of the first expected hit in the top 10.
- `P@k`: relevant top-`k` results divided by returned top-`k` results.
- `R@k`: expected evidence refs matched by top-`k` results.
- `nDCG@10`: binary ranking quality against the expected evidence count.
- `evidence@k`: expected evidence refs matched by top-`k` results.
- `Abstention`: no-answer / false-premise queries where returning no curated result is the desired behavior.

## Schema

Top-level fields:

- `version`: schema version string.
- `description`: human-readable dataset note.
- `corpus`: optional fixture memories. When present, `remem eval` seeds these memories into an in-memory SQLite database and does not open the configured live database.
- `queries`: array of query cases.

Corpus memory fields:

- `project`: project path or stable synthetic project id.
- `topic_key`: optional stable topic key.
- `title`: memory title.
- `content`: memory text to seed.
- `memory_type`: memory type, for example `decision`, `discovery`, `procedure`, or `lesson`.
- `branch`: optional branch filter value.
- `scope`: optional memory scope. Defaults to `project`.
- `status`: optional lifecycle status. Defaults to `active`.
- `files`: optional JSON-encoded file list.
- `created_at_epoch`: optional fixed creation timestamp.

Query fields:

- `id`: stable case id.
- `query`: user-facing search query.
- `category`: bucket for per-category reporting, for example `single_session`, `multi_session`, `temporal`, `knowledge_update`, `project_scope`, `procedure`, or `abstention`.
- `slice`: ability slice for per-slice reporting, for example `paraphrase`, `knowledge_update`, `temporal`, `abstention`, `failure_lesson`, `multi_hop`, or `associative`. Defaults to `category` for older datasets.
- `hop_path`: optional documented query -> entity -> target path. Required for `slice: "associative"` and validated by the loader.
- `project`: optional project filter.
- `branch`: optional branch filter.
- `memory_type`: optional memory type filter.
- `evidence_refs`: stable expected evidence references. Prefer this for new cases.
- `relevant_ids`: legacy memory-id list. Still accepted, but less stable than evidence refs.
- `expect_abstain`: true when no curated memory should be returned.
- `false_premise`: true for adversarial queries based on a false premise. This also counts as abstention.
- `notes`: optional maintenance note.

Evidence ref fields are conjunctive: every populated field must match the returned memory.

- `memory_id`: legacy exact memory id.
- `topic_key`: stable topic key.
- `project`: expected project.
- `branch`: expected branch.
- `memory_type`: expected memory type.
- `scope`: expected memory scope.
- `title_contains`: case-insensitive title substring.
- `text_contains`: case-insensitive memory text substring.

Associative `hop_path` fields:

- `source`: topic key for the intermediate fixture memory.
- `entity_type`: one of `file_path`, `crate`, `error_signature`, or `issue_number`.
- `entity`: linking entity expected in both source and target memories.
- `target`: topic key for the judged-relevant target memory.

Example:

```json
{
  "id": "procedure-pr-review",
  "query": "PR review merge workflow",
  "category": "procedure",
  "project": "tools/remem",
  "branch": "main",
  "evidence_refs": [
    {
      "topic_key": "pr-review-merge-workflow",
      "memory_type": "procedure",
      "text_contains": "@codex review"
    }
  ]
}
```

## Extraction Quality Eval

`remem eval-extraction` runs a deterministic extraction-quality check against a
labeled transcript corpus and committed parser/model-output baseline.

```bash
remem eval-extraction --json --check-baseline
```

The corpus lives in `eval/extraction/corpus.json`; the committed baseline report
lives in `eval/extraction/baseline.json`. CI runs the command above, so prompt,
parser, replay fixture, and label changes that affect extraction metrics or
request fingerprints must update the baseline intentionally.

The JSON report includes:

- observation precision and recall
- memory-candidate precision and recall
- forbidden-label exclusion rates
- over-saved prediction count and over-save penalty
- observation and candidate replay request SHA-256 fingerprints
- per-case missing, unexpected, and forbidden predictions

## Eval Regression Gates

`remem eval-gates` runs the CI regression gate for golden retrieval,
capacity degradation, SessionStart injection, and aggregate extraction quality:

```bash
remem eval-gates --json-out /tmp/remem-eval-gates.json
```

The gate compares current deterministic eval metrics with
`eval/gates/baseline.json` using thresholds from `eval/gates/thresholds.json`.
It prints a delta table in CI and writes the full JSON artifact, including the
source eval reports. Golden eval artifacts include per-slice estimated
tokens/query plus p50/p95 retrieval latency for trend inspection; latency is not
used as a hard gate. Capacity artifacts include the fused degradation curve and
per-channel loss metrics for `fts`, `entity`, `fact`, `temporal`, `vector`, and
`like_fallback`; quality-loss increases are gated through the thresholds file.
CI also keeps the exact `eval-extraction --check-baseline` gate so extraction
prompt, parser, replay fixture, and request-fingerprint changes cannot pass on
aggregate rates alone.

The hidden `--simulate-golden-regression` and
`--simulate-capacity-regression` flags are exercised in CI to prove the gate
fails on constructed retrieval and capacity regressions before changing
defaults.

## Provider Comparison Eval

`remem eval-provider-comparison` runs the GH-716 default-flip evidence report
for the embedding providers:

```bash
REMEM_DATA_DIR=eval/provider-comparison/reference-data \
  remem eval-provider-comparison --json-out eval/provider-comparison/report.json
```

The report forces `feature-hash`, `local`, and `api` rows without fallback so
one provider cannot pass by silently using another provider's vector space. API
embedding calls are disabled by default; pass `--allow-api` only for an
intentional remote-provider run. Missing local model files or skipped API calls
produce unavailable rows, not degraded passes.

The checked-in GH-716 report keeps the default provider unchanged. It records
feature-hash as the runnable baseline, local semantic as blocked by a missing
verified model manifest in the reference data dir, and API as blocked because
the committed reference run does not call remote embeddings.

## Capacity Eval

`remem eval-capacity` runs the first deterministic capacity curve for issue #675.
It grows the committed golden fixture with seeded, non-relevant synthetic
memories and reports fused plus per-channel retrieval metrics at each requested
scale:

```bash
remem eval-capacity --seed 42 --scales 1,10 --json-out /tmp/remem-capacity.json --json
```

The JSON artifact records the seed, scale factors, corpus size, synthetic noise
count, SHA-256 corpus hash, fused metrics, channel metrics, p95 retrieval
latency, and loss against the 1x baseline. The committed gate currently runs
1x/10x and enforces zero quality-loss increase for fused and channel-level
degradation metrics. Dashboard ingestion and 50x nightly scheduling remain
follow-up work under #675.

## Associative Multi-Hop Baseline

`remem eval-associative-baseline` runs the first fixture-quality slice for
issue #676. It filters `slice: "associative"` from the committed golden corpus,
checks entity-class coverage and query-target lexical leakage, then writes the
baseline fused metrics and headroom report:

```bash
remem eval-associative-baseline --json-out eval/associative-multihop/baseline.json --json
```

The checked-in report demonstrates that the associative slice has baseline
retrieval headroom. It intentionally does not include per-channel attribution,
entity-BFS deltas, literal `graph_edges` traversal, or the ADR follow-up
decision; those remain follow-up work under #676.

## Graph Decision Gate

`remem eval-graph-decision` runs the issue #382 wire-or-freeze gate. It compares
standard golden retrieval against the explicit entity-BFS multi-hop proxy path
and writes the artifact used by the graph retrieval ADR:

```bash
remem eval-graph-decision --json-out eval/graph-decision/report.json
```

The gate records the pre-registered 5% multi-hop evidence-recall threshold,
non-`multi_hop` zero-regression checks, a 1000ms p95 latency budget, whether the
entity-BFS proxy exercised two-hop expansion, and whether literal `graph_edges`
traversal was evaluated. A failure to clear the entity-BFS wire requirements is
not a process failure by itself; it means the correct decision is to keep
`graph_edges` frozen as a retrieval channel until a future literal graph-edge
fixture and A/B report prove material value.
