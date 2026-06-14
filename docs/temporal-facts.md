# Temporal Facts

remem uses a lightweight SQLite fact layer for time-sensitive relationships.
This layer is deliberately relational: it adds queryable edges without adopting
a graph database or vector index.

## Schema

Migration `v013_memory_temporal_facts` creates `memory_facts`:

| Column | Meaning |
|---|---|
| `project` | Project path/scope used by existing memory retrieval. |
| `subject` | Stable local subject key, such as `deploy-target`, `PR #190`, or a file path. |
| `predicate` | One fixed coding relation: `fixed_by`, `verified_by`, `supersedes`, `blocked_by`, `uses_file`, `uses_command`, `affects_project`. |
| `object` | Relation target, stored as text so it can point to commands, files, issues, PRs, or memory ids. |
| `valid_from_epoch` / `valid_to_epoch` | Validity interval for the fact. `valid_to_epoch` is exclusive. |
| `learned_at_epoch` | When remem learned the fact; separate from validity time. |
| `invalidated_at_epoch` | Transaction time when remem learned the fact stopped being current. `NULL` means still current. |
| `source_memory_id` | Optional link to the durable memory that produced the fact. |
| `source_observation_id` | Optional link to the observation that produced the fact. |
| `source_event_ids` | JSON array of raw `captured_events` ids. |
| `confidence` | Extractor/reviewer confidence, constrained to `0.0..=1.0`. |
| `supersedes_fact_id` | Optional pointer to the fact this one replaced. |
| `status` | `active` for current facts, `stale` after soft supersession. |

## Query Path

Current-fact queries filter by project, optional subject, optional predicate,
`status='active'`, `invalidated_at_epoch IS NULL`, and the current validity
interval. Historical queries use `list_facts_as_of(project, as_of_epoch, ...)`
and include rows only when remem had learned them and had not invalidated them
at the requested transaction time.

This split is intentional:

- default retrieval should not surface obsolete facts as current truth
- historical/debug flows can still answer "what was true at the time?"
- superseded facts remain auditable through source ids and validity windows

## Supersession

Inserting a fact with `supersedes_fact_id` performs one transaction:

1. verify the superseded fact exists in the same project
2. mark the old fact `stale`
3. set the old `valid_to_epoch` to the replacement's `valid_from_epoch`, or to
   `learned_at_epoch` when validity is unknown
4. set the old `invalidated_at_epoch` to the transaction time
5. insert the replacement as `active`

The old fact is preserved rather than deleted. This mirrors the memory lifecycle
rule that invalidation is a soft state transition unless a future privacy or
retention feature explicitly requires hard deletion.

## Design Boundary

This model is the first step toward temporal relationship memory. It does not
try to solve entity merging, PageRank, community summaries, graph traversal, or
LLM extraction in this PR. Those can build on the table once the write and query
semantics are proven.
