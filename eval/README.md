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
- `memory_type`: memory type, for example `decision`, `discovery`, or `procedure`.
- `branch`: optional branch filter value.
- `scope`: optional memory scope. Defaults to `project`.
- `status`: optional lifecycle status. Defaults to `active`.
- `files`: optional JSON-encoded file list.
- `created_at_epoch`: optional fixed creation timestamp.

Query fields:

- `id`: stable case id.
- `query`: user-facing search query.
- `category`: bucket for per-category reporting, for example `single_session`, `multi_session`, `temporal`, `knowledge_update`, `project_scope`, `procedure`, or `abstention`.
- `slice`: ability slice for per-slice reporting, for example `paraphrase`, `knowledge_update`, `temporal`, `abstention`, or `multi_hop`. Defaults to `category` for older datasets.
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
