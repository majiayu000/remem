# Truthful MCP Tool Metadata — Product Spec

Refs #981.

## Problem

remem exposes 14 MCP tools, but their descriptors currently omit tool
annotations and output schemas. MCP clients therefore have to guess whether a
call mutates durable state, can be repeated safely, may contact an external
provider, or returns machine-readable JSON. The protocol defaults are also
materially wrong for several tools: read-only queries appear destructive,
while detail reads and poisoning gates hide their persistent side effects.

The public tool names and legacy text responses are already in use. Metadata
must become truthful without forcing existing clients to adopt a new response
shape.

## Scope

- Publish an explicit title and all four MCP hints for every registered tool:
  `readOnlyHint`, `destructiveHint`, `idempotentHint`, and `openWorldHint`.
- Publish an object-rooted `outputSchema` for every JSON-producing tool.
- Emit JSON Schema 2020-12-compatible null unions; never publish the
  non-standard OpenAPI `nullable` keyword.
- Add matching `structuredContent` to successful JSON responses while
  preserving the existing text content byte-for-byte.
- Keep `timeline_report` as Markdown text with no misleading JSON schema.
- Fail server construction if a registered tool has no contract or a contract
  names a tool that is not registered.
- Fail a schema-bearing successful call if its legacy content is not the
  expected JSON top-level shape; do not silently omit structured output.
- Pin the final merged router metadata and response-adapter behavior in tests.

## Side-Effect Boundary

The annotations describe persistent user/business state and external provider
interaction:

- Durable memory, summary quarantine, workstream, audit, job, and access
  metadata writes count as mutations.
- Overwrite, quarantine, or state-transition paths are destructive. Purely
  additive writes would not be destructive, but no current write tool is
  guaranteed additive across all parameter combinations.
- A repeated call is idempotent only when it does not add or advance a durable
  effect after the first call.
- Optional remote embedding requests make a tool open-world even when its
  default or another branch is local-only.
- Normal server-start database migration, diagnostic logging, local model
  cache/lock maintenance, and provider quota accounting are outside the
  durable-domain mutation boundary. They must not be used to conceal a tool's
  actual domain write.

## Normative Tool Matrix

`R`, `D`, `I`, and `O` below map to the four MCP hints in that order.

| Tool | Title | R | D | I | O | Reason |
|---|---|---:|---:|---:|---:|---|
| `current_state` | Current State | true | false | true | false | Local current-state query only. |
| `search` | Search Memories | true | false | true | true | Query-only; a query may use a remote embedding provider. |
| `recall_user_context` | Recall User Context | false | true | false | true | May quarantine an unsafe summary and may use remote embedding. |
| `timeline` | Memory Timeline | true | false | true | true | Query-only; query mode may use remote embedding. |
| `get_observations` | Get Observation Details | false | true | false | false | Overwrites last-accessed metadata and increments memory access counts. |
| `lookup_commit` | Lookup Commit | false | true | false | false | Its safety gate may quarantine the newest eligible linked summary. |
| `commits_for_session` | List Session Commits | false | true | false | false | Its safety gate may quarantine the newest eligible linked summary. |
| `save_memory` | Save Memory | false | true | false | true | Inserts or overwrites durable records/local copies and may embed remotely. |
| `govern_memory` | Govern Memory | false | true | false | false | Non-dry-run actions transition state and append audit/job records. |
| `timeline_report` | Timeline Report | true | false | true | false | Local aggregate query returning Markdown. |
| `workstreams` | List Workstreams | true | false | true | false | Local workstream query only. |
| `update_workstream` | Update Workstream | false | true | false | false | Overwrites mutable fields and timestamps on an existing workstream. |
| `search_raw` | Search Raw Archive | true | false | true | false | Local raw-archive query only. |
| `list_raw_sessions` | List Raw Sessions | true | false | true | false | Local raw-session aggregate query only. |

Static annotations cover the most permissive or hazardous path. A dry-run or
anchor-only invocation does not weaken a tool descriptor whose other valid
parameters can write state or contact an external provider.

## Output Compatibility

Successful JSON tools retain their existing single text content item exactly.
They additionally return object-rooted structured content:

- Existing JSON objects are copied directly: `current_state`, `search`,
  `recall_user_context`, `save_memory`, `govern_memory`, `update_workstream`,
  `search_raw`, and `list_raw_sessions`.
- Existing JSON arrays keep the array in text content and use a named object
  envelope only in structured content:
  - `timeline` → `{ "observations": [...] }`
  - `get_observations` → `{ "details": [...] }`
  - `lookup_commit` → `{ "commits": [...] }`
  - `commits_for_session` → `{ "commits": [...] }`
  - `workstreams` → `{ "workstreams": [...] }`
- `timeline_report` remains one Markdown text item and has neither
  `outputSchema` nor `structuredContent`.
- Error results retain the existing structured-error JSON text contract and
  do not claim successful `structuredContent`.

No public tool name, input schema, text serialization, ordering, whitespace,
or error payload changes as part of #981.

## Non-Goals

- No handler behavior, database schema, retrieval policy, or embedding-provider
  change.
- No conversion of `timeline_report` from Markdown to JSON.
- No removal of access accounting or poisoning quarantine to make a tool look
  read-only.
- No automatic Glama rescan, release, merge, or external registry mutation.

## Success Criteria

- The final merged router contains exactly 14 tools and a complete, exact
  contract for each one.
- All 14 tools expose explicit annotations matching the matrix.
- Thirteen JSON tools expose stable object-rooted output schemas; the one
  Markdown tool intentionally does not.
- Normal successful responses containing JSON `null` validate against the
  published 2020-12 schemas.
- Representative object and array calls prove legacy text preservation and
  schema-conforming structured content; errors never masquerade as success.
- Focused MCP tests, formatting, `cargo check`, and the repository preflight
  pass from the submitted commit.
- After the implementation is deployed and Glama refreshes the server, its
  public introspection shows all 14 tool descriptors with the same annotations
  and output-schema presence. An empty or stale Glama tool list is external
  pending evidence, not a reason to weaken the local contract.
