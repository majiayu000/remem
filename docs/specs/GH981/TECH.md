# Truthful MCP Tool Metadata — Tech Spec

Refs #981.

## Design

The six macro-generated routers remain responsible for tool names, input
schemas, descriptions, and handlers. A single contract layer decorates their
fully merged `ToolRouter<MemoryServer>` during `MemoryServer::new`:

```text
six generated routers
        |
        v
merged ToolRouter<MemoryServer>
        |
        v
apply exact 14-entry contract table
  - title + four annotations
  - optional object-rooted output schema
  - optional success-response adapter
        |
        v
MCP list_tools / call_tool wire surface
```

This keeps contract completeness auditable in one place and avoids changing
the 14 handler return types. Existing direct handler tests continue to observe
their current `McpToolResult<String>` contract.

## Module Layout

```text
src/mcp/server.rs
src/mcp/server/
  tool_contracts.rs          exact registry, router decoration, adapter
  tool_contracts/
    schemas.rs               output-only schema DTOs
  tests/tool_metadata.rs     final-router and compatibility tests
```

The implementation may split schema DTOs further if needed to keep source
files under the repository's size ceiling. It must not duplicate a second
tool-name registry outside `tool_contracts` and the exact-set assertion in
tests.

## Contract Registry

Each entry contains:

- exact tool name and human-readable title;
- explicit booleans for all four `ToolAnnotations` hints;
- `None` for Markdown or a schema builder for JSON output;
- expected legacy top-level shape (`Object` or `Array`);
- array envelope key when the legacy top level is an array.

Application is fail-closed:

1. Build the six routers and merge them.
2. Compare the registered-name set with the contract-name set.
3. Return an initialization error that reports missing and unexpected names
   when the sets differ.
4. Apply annotations and output schema to each matching route.
5. Wrap only schema-bearing call handlers with the success adapter.

The registry applies metadata after router merging so tests and protocol
listing inspect exactly the routes that are served.

## Wire Names

Rust uses the rmcp model fields; JSON serialization must expose the MCP wire
names:

- `readOnlyHint`
- `destructiveHint`
- `idempotentHint`
- `openWorldHint`
- `outputSchema`
- `structuredContent`

Tests serialize the final descriptors/results and assert the camelCase wire
keys. No snake_case alias is introduced at the API boundary.

## Output Schemas

Every published schema has JSON Schema `type: "object"` at the root. Output
DTOs describe the stable fields currently emitted by the handlers and permit
documented conditional fields where a tool has multiple modes. The schema
must not promise a field or enum value that production cannot emit; in
particular, `current_state.status` includes `no_current`.

Every typed object with declared `properties` publishes
`additionalProperties: false`, including nested DTO definitions. Deliberately
dynamic `serde_json::Value` extension points such as search explanation data
remain unconstrained; the contract must not accidentally close data whose
shape is owned by a separately versioned subsystem.

rmcp 0.15's schema transform emits the OpenAPI-style `nullable` keyword even
though its schema declares JSON Schema 2020-12. `build_schema` therefore
normalizes every generated output schema before publication: a typed nullable
node becomes a `type` union containing `null`, and a nullable reference or
composition becomes an `anyOf` with `{ "type": "null" }`. No published output
schema may contain `nullable`. Fields that are always serialized but may be
null, such as governance `reason`, remain in `required` after normalization.

| Tool | Legacy text top level | Structured root | Required stable fields |
|---|---|---|---|
| `current_state` | object | same object | `status` |
| `search` | object | same object | `mode`, `results`, `next_step` |
| `recall_user_context` | object | same object | `query`, `context`, `included`, `dropped`, `diagnostics` |
| `timeline` | array | `observations` | `observations` |
| `get_observations` | array | `details` | `details` |
| `lookup_commit` | array | `commits` | `commits` |
| `commits_for_session` | array | `commits` | `commits` |
| `save_memory` | object | same object | `status`, `operation`, `next_step` |
| `govern_memory` | object | same object | `dry_run`, `action`, `reason`, `affected` |
| `workstreams` | array | `workstreams` | `workstreams` |
| `update_workstream` | object | same object | `id`, `updated` |
| `search_raw` | object | same object | `query`, `count`, `has_more`, `results` |
| `list_raw_sessions` | object | same object | `sample`, `count`, `sessions` |
| `timeline_report` | Markdown | none | no JSON schema |

Nested result items use output-specific schema DTOs or explicit schema
objects. Input DTO reuse is allowed only when serialization proves the output
shape is identical. Dynamic, documented nested extension objects may use a
JSON value schema, but the root and stable fields above may not be reduced to
an unconstrained value.

## Success Adapter

The wrapper invokes the original route handler and then:

1. Returns error results unchanged with no structured content.
2. Requires exactly one text content item for a successful schema-bearing
   result.
3. Parses that text as JSON without rewriting the original content.
4. Validates the expected object/array top-level shape.
5. Copies objects directly or wraps arrays under the registry envelope key.
6. Deserializes the structured value through the exact output DTO used to
   generate that route's schema. Every typed DTO rejects undeclared fields;
   required, nullable, enum, nested, and untagged-union semantics therefore
   remain executable on the served path.
7. Sets `structuredContent` and returns the original content unchanged.

Unexpected content count/type, malformed JSON, wrong top-level shape, or DTO
validation failure is an internal MCP error naming the tool and contract
violation. It must never be a warning plus missing structured content.

No second generic JSON-Schema interpreter is added to the hot path. Schema
generation and runtime validation share one output DTO per route, preventing
the two contract descriptions from drifting independently.

## Description Corrections

Descriptions remain concise but must agree with the annotations:

- `recall_user_context` discloses that its poisoning gate may quarantine an
  unsafe legacy summary; it must not call itself read-only.
- `lookup_commit` and `commits_for_session` disclose the same linked-summary
  quarantine possibility.
- `get_observations` continues to disclose last-access/access-count writes.
- Query tools may still say read-only when they do not mutate durable domain
  state; their `openWorldHint` separately reports optional remote embedding.

## Tests

Focused tests cover:

- exact equality of registered and contracted 14-tool name sets;
- the full title/R/D/I/O matrix from the final merged router;
- output-schema presence for exactly 13 tools and absence for
  `timeline_report`;
- object-rooted schemas with the stable fields/envelope keys above;
- closed root and nested typed objects, with documented dynamic extension
  values left open;
- serialized descriptor camelCase wire keys;
- object adaptation preserves the original content bytes;
- array adaptation preserves the original array text and exposes the named
  structured envelope;
- error results remain unchanged without structured content;
- malformed or wrong-shape successes fail loudly;
- undeclared, missing, wrong-type, invalid-null, nested, and union-branch
  mutations fail against the advertised contract;
- every one of the 13 schema-bearing served routes returns a real non-empty
  success through the adapter, covering both detail union branches and
  nested/nullable values;
- `MemoryServer::new` remains lazy and does not open the database.

Existing handler tests remain the behavioral oracle for the legacy text
response. Verification uses focused MCP tests first, then `cargo fmt --check`,
`cargo check`, `cargo test`, and the PR preflight.

## Version and Release Metadata

The implementation is a user-visible MCP wire enhancement and receives one
unique staged source version. The version must stay synchronized across
`Cargo.toml`, `Cargo.lock`, the Codex plugin manifest, runtime release
manifest, npm wrapper, app server metadata, and `CHANGELOG.md` according to
the repository version-sync checks.

## Registry Verification

Local tests are authoritative for the submitted code. After a release is
available to Glama, fetch its MCP server introspection and verify all 14 tool
names, annotations, and output-schema presence. If Glama still reports an
empty or stale list, retain that as explicit external pending evidence and
coordinate the registry refresh separately; do not alter the truthful local
metadata to satisfy a stale scan.
