# Truthful MCP Tool Metadata — Product Spec

Refs #981, #932, #1061.

## Problem

remem exposes 15 MCP tools. The original 14-tool metadata rollout established
the contract below; GH932 subsequently added the experimental `context_bundle`
JSON tool under the same fail-closed registry and wire rules. Tool descriptors
must not omit annotations and output schemas. MCP clients otherwise have to guess whether a
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
- Close every typed object schema against undeclared fields while leaving only
  documented dynamic extension values unconstrained.
- Emit JSON Schema 2020-12-compatible null unions; never publish the
  non-standard OpenAPI `nullable` keyword.
- Add matching `structuredContent` to successful JSON responses while
  preserving the existing text content byte-for-byte.
- Keep `timeline_report` as Markdown text with no misleading JSON schema.
- Fail server construction if a registered tool has no contract or a contract
  names a tool that is not registered.
- Fail a schema-bearing successful call if its legacy content does not conform
  to the exact DTO that generated its advertised schema, including required,
  nullable, nested, union, enum, and closed-object constraints.
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
| `context_bundle` | Compile Context Bundle (Experimental) | false | true | false | false | Uses local-only retrieval, but poisoning checks may persist audit/drop records. |
| `timeline` | Memory Timeline | true | false | true | true | Query-only; query mode may use remote embedding. |
| `get_observations` | Get Observation Details | false | true | false | false | Memory and observation reads update access metadata; session-summary reads may quarantine unsafe generated text. |
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
  `recall_user_context`, `context_bundle`, `save_memory`, `govern_memory`,
  `update_workstream`, `search_raw`, and `list_raw_sessions`.
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

The detail union accepts `source=memory`, `source=observation`, and the exact-ID
`source=session_summary` branch used by prompt-time continuity anchors. No
public tool name, text serialization, ordering, whitespace, or error payload
changes as part of #981.

## Contract Amendment (#1061)

GH981 remains the canonical MCP tool contract. #1061 is a reviewed breaking
amendment to three production tools on that contract, not a parallel MCP spec:

- `save_memory`: pass `host` when the calling host is known. An omitted host is
  stored as `unknown`. The server does not infer `codex-cli`.
- `govern_memory`: call `dry_run=true` first. The preview's `expected_versions`
  are the versions loaded inside that governance transaction. Non-dry-run
  mutations require `expected_versions` for every target ID, plus
  `confirm_destructive=true` and an explicit reason. A later SELECT must not
  authorize a mutation of a state the caller never previewed.
- `recall_user_context`: `project` or `cwd` is required. The server does not
  infer this scope from its own process working directory.

Other tools keep the GH981 legacy text shapes. `govern_memory` dry-run text
additionally includes `expected_versions`, and each `affected` item includes
the transaction-bound `version`. Those fields are additive JSON; omitting
`expected_versions` on a mutation is an invalid request.

## Non-Goals

- No handler behavior, database schema, retrieval policy, or embedding-provider
  change.
- No conversion of `timeline_report` from Markdown to JSON.
- No removal of access accounting or poisoning quarantine to make a tool look
  read-only.
- No automatic Glama rescan, release, merge, or external registry mutation.

## Success Criteria

- The final merged router contains exactly 15 tools and a complete, exact
  contract for each one.
- All 15 tools expose explicit annotations matching the matrix.
- Fourteen JSON tools expose stable object-rooted output schemas; the one
  Markdown tool intentionally does not.
- Typed output objects reject undeclared fields instead of advertising an open
  shape broader than the production response.
- Normal successful responses containing JSON `null` validate against the
  published 2020-12 schemas.
- Real non-empty served-wire successes from all fourteen JSON tools prove
  legacy text preservation and schema-conforming structured content, including
  all three detail union branches and nested/nullable values; errors never
  masquerade as success.
- Focused MCP tests, formatting, `cargo check`, and the repository preflight
  pass from the submitted commit.
- After the implementation is deployed and Glama refreshes the server, its
  public introspection shows all 15 tool descriptors with the same annotations
  and output-schema presence. An empty or stale Glama tool list is external
  pending evidence, not a reason to weaken the local contract.
