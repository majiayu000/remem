# Graph Contract

remem keeps graph memory SQLite-first. The graph contract adds typed references
and provenance rules without adopting a graph database or changing retrieval
behavior.

## Scope

`memory_edges` remains the memory-to-memory lifecycle table. Existing APIs and
queries keep using it for replacement, merge, split, duplicate, conflict, and
provenance summaries on durable memories.

`graph_edges` is the first-class cross-node contract for future traversal. It
can connect memory, entity, fact, episode, state, topic, and file nodes, but
this PR only defines storage and Rust insertion types. Search, context
injection, and retrieval ranking do not read `graph_edges` yet.

## Node References

Graph node references are typed by `(node_kind, node_id)`.

| Node kind | Backing table | Meaning |
|---|---|---|
| `memory` | `memories(id)` | Durable curated memory row. |
| `entity` | `entities(id)` | Canonical extracted entity. |
| `fact` | `memory_facts(id)` | Temporal fact with validity and provenance. |
| `episode` | `captured_events(id)` | Raw capture-ledger event used as episode evidence. |
| `state` | `memory_state_keys(id)` | Stable mutable-memory state slot. |
| `topic` | `topic_segments(id)` | Topic Loom segment for continuity/evidence. |
| `file` | `graph_file_nodes(id)` | Project-scoped file path observed in evidence. |

Rust code must construct refs through `GraphNodeRef` so invalid ids fail before
SQL execution. The database also has insert/update triggers that reject missing
backing rows for every node kind, plus delete triggers that remove graph edges
when a referenced node is hard-deleted. This is intentional: ad hoc SQL should
fail closed rather than leave dangling graph references.

## Edge Storage

Migration `v031_graph_edges` creates `graph_edges`:

| Column | Meaning |
|---|---|
| `edge_type` | Fixed graph relation vocabulary. |
| `edge_trust` | `trusted` or `diagnostic_hint`, derived from `edge_type`. |
| `from_node_kind` / `from_node_id` | Typed source node ref. |
| `to_node_kind` / `to_node_id` | Typed target node ref. |
| `source_event_ids` | JSON array of positive `captured_events.id` evidence. |
| `source_candidate_id` | Candidate row that proposed the trusted edge; may point to a graph or memory candidate depending on the writer. |
| `source_operation_id` | Operation log row that accepted/wrote the edge. |
| `confidence` | Extractor/reviewer confidence, constrained to `0.0..=1.0`. |
| `reason` | Human/auditable reason for the edge. |
| `valid_from_epoch` / `valid_to_epoch` | Optional validity interval for relations whose truth changes. |
| `created_at_epoch` | Insertion time. |

Trusted edges require non-empty `source_event_ids`, `source_candidate_id`,
`source_operation_id`, `confidence`, `reason`, and `created_at_epoch`.
`source_event_ids` must be a non-empty JSON array of existing captured event
ids. If a source event is later hard-deleted, graph edges using it as trusted
evidence are removed with it.
Diagnostic hints may omit provenance because they are not authoritative inputs
to retrieval or memory truth.

## Endpoint Constraints

Each edge type has fixed endpoint kinds. Rust `insert_graph_edge` validates the
same matrix that the schema enforces for raw SQL writers:

| Edge types | Allowed endpoints |
|---|---|
| `supersedes`, `duplicates`, `conflicts`, `derived_from`, `merged_into`, `split_from` | Same node kind on both sides. |
| `extracted_from` | `entity`, `fact`, `state`, or `topic` to `episode`. |
| `mentions` | `memory` or `episode` to `entity`. |
| `touches_file` | `memory` or `episode` to `file`. |
| `has_state` | `memory` to `state`. |
| `has_topic` | `memory` or `episode` to `topic`. |
| `similar_to`, `candidate_hint`, `co_occurs_with` | Same node kind on both sides. |

## Edge Types

Trusted graph edges:

| Edge type | Intended use |
|---|---|
| `supersedes` | Current node replaces an older node. |
| `duplicates` | Nodes represent the same durable claim. |
| `conflicts` | Nodes disagree and need review or temporal resolution. |
| `derived_from` | Node was derived from another durable node. |
| `merged_into` | Node was merged into a replacement. |
| `split_from` | Node was split from a broader source. |
| `extracted_from` | Fact/entity/state/topic was extracted from episode evidence. |
| `mentions` | Memory or episode mentions an entity. |
| `touches_file` | Memory or episode touches a project file path. |
| `has_state` | Memory belongs to a stable state key. |
| `has_topic` | Memory or episode belongs to a topic segment. |

Diagnostic hints:

| Edge type | Intended use |
|---|---|
| `similar_to` | Approximate similarity, embedding neighbor, or heuristic match. |
| `candidate_hint` | Extractor candidate link that has not been promoted. |
| `co_occurs_with` | Co-occurrence signal useful for debugging or review. |

Diagnostic hints must not be treated as trusted retrieval edges without a later
promotion operation that writes a trusted edge with full provenance.

## Compatibility

The graph contract is additive:

- existing `memory_edges` writes and summaries continue unchanged
- `memory_facts` remains the temporal fact table
- `memory_state_keys` remains the mutable state identity table
- `topic_segments` remains the topic continuity/evidence table
- no search or context behavior reads `graph_edges` yet

Future traversal work should build on `GraphNodeRef`, `GraphEdgeType`,
`GraphEdgeProvenance`, and `insert_graph_edge` instead of writing stringly typed
node refs or raw edge type strings in new code.
