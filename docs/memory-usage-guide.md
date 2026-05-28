# Memory Usage Guide

remem keeps MCP tool descriptions short so agents see contracts first and
tutorials only when they need them.

## Retrieval

Use `search(query, project?)` first. It returns compact results with IDs,
source labels, pagination, and the next `get_observations` call to make.

Use `get_observations(ids, source)` for selected full details. Do not fetch
every result by default.

Use `search_raw(query)` when curated search is empty or too sparse and you need
literal chat recall. Raw hits are transcript evidence, not curated memory.

For complex questions:

1. Break the question into two or three focused searches.
2. If the first pass is sparse, search again with concrete names or files from
   the results.
3. Use `multi_hop=true` when the question spans multiple people, projects, or
   topics.

## Persistence

Use `save_memory` for durable information:

- technical decisions and rejected alternatives
- bugfix root causes and prevention notes
- important discoveries or constraints
- architecture notes
- user preferences

Before saving, search for an existing topic and reuse a stable kebab-case
`topic_key` when the memory updates a previous fact.

If the user asks to save, write, or update a document, create or edit the local
project file first. `save_memory` is only a long-term backup.

## Workstreams

Use `workstreams(project)` to list active tracked work. Use
`update_workstream(id, status?, next_action?, blockers?)` to update status,
next action, or blockers.
