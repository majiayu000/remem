# SPEC: remem-web local REST API

Status: current contract. Refs #568, #825.

The native web API is the localhost, bearer-token boundary used by remem-web
and Apps SDK clients. Clients must not read the SQLCipher database directly or
invent mock graph/candidate data when the native binary lacks an endpoint.

The complete web read-model surface is implemented in source version
`0.5.109`. Fast health checks and cached status metadata are implemented in
source version `0.5.112`. Suppression audit opt-in is implemented in source
version `0.5.113`. Task-aware user recall is implemented in source version
`0.5.114`. User recall usage-policy guidance is implemented in source version
`0.5.123`. remem-web should require a published release with the specific
capability it needs before directing installed-binary users to that surface.
Clients should call `GET /api/v1/capabilities` before enabling optional UI
features.
Candidate review queue filters and the blocked-reason aggregate endpoint are
implemented in source version `0.5.162`.
The GH-880 safe console contract is implemented in source version `0.6.6`:
candidate detail/evidence and idempotent safe review, five independently gated
safe read resources, and recoverable memory archive/restore. This source
version remains unavailable to installed-binary clients until a corresponding
release is published.

## Endpoint Groups

### Stable Core

| Method | Path | Purpose |
|---|---|---|
| GET | `/api/v1/health` | Cheap authenticated liveness and API readiness. |
| GET | `/api/v1/status` | Cached operational queue state and counters. |
| GET | `/api/v1/capabilities` | Native feature and endpoint discovery. |
| GET | `/api/v1/search?query=&project=&type=&limit=&offset=&branch=&include_stale=&include_suppressed=&multi_hop=&explain=` | Search memories with optional explain. |
| GET | `/api/v1/memory?id=&include_suppressed=` | Legacy compact single-memory endpoint. |
| GET | `/api/v1/memories?project=&type=&scope=&status=&branch=&q=&limit=&offset=&include_suppressed=` | Canonical browse endpoint. |
| GET | `/api/v1/memories/{id}?include_suppressed=` | Rich detail with entities and memory edges. |
| POST | `/api/v1/memories` | Explicit durable memory save. |
| POST | `/api/v1/user/recall` | Task-aware user-context recall with source and drop reasons. |

### Web Read Model

| Method | Path | Purpose |
|---|---|---|
| GET | `/api/v1/stats` | Product stats for local dashboards. |
| GET | `/api/v1/candidates?project=&status=&type=&block_reason=&topic_key=&contains=&min_confidence=&older_than_days=&limit=&offset=` | Compact memory-candidate list with review filters. |
| GET | `/api/v1/candidates/blocked?project=` | Pending candidate block-reason aggregates with examples. |
| GET | `/api/v1/candidates/{id}` | Safe candidate detail, evidence projection, and review decision. |
| POST | `/api/v1/candidates/{id}/review/approve` | Versioned, audited, idempotent safe approval. |
| POST | `/api/v1/candidates/{id}/review/reject` | Versioned, audited, idempotent safe rejection. |
| POST | `/api/v1/candidates/{id}/review/edit` | Versioned, audited, idempotent safe edit-and-approve. |
| POST | `/api/v1/candidates/{id}/approve` | Approve a pending candidate. |
| POST | `/api/v1/candidates/{id}/reject` | Reject a pending candidate; persisted status is `discarded`. |
| POST | `/api/v1/candidates/{id}/edit` | Edit and approve a pending candidate. |
| GET | `/api/v1/graph?project=&limit=&include_suppressed=` | DB-backed entity graph. |
| GET | `/api/v1/observations?page_size=&cursor=&project=` | Safe observation list with typed keyset cursor. |
| GET | `/api/v1/observations/{id}` | Safe observation detail. |
| GET | `/api/v1/sessions?page_size=&cursor=&project=` | Safe session list with typed keyset cursor. |
| GET | `/api/v1/sessions/{id}` | Safe session detail. |
| GET | `/api/v1/workstreams?page_size=&cursor=&project=` | Safe workstream list with typed keyset cursor. |
| GET | `/api/v1/workstreams/{id}` | Safe workstream detail. |
| GET | `/api/v1/events?page_size=&cursor=&project=` | Safe captured-event metadata list; raw content is excluded. |
| GET | `/api/v1/events/{id}` | Safe captured-event metadata detail. |
| GET | `/api/v1/tasks?page_size=&cursor=&project=` | Safe extraction-task list; payload and raw errors are excluded. |
| GET | `/api/v1/tasks/{id}` | Safe extraction-task detail. |
| POST | `/api/v1/memories/{id}/archive` | Recoverably archive an active memory. |
| POST | `/api/v1/memories/{id}/restore` | Restore only the current exact Web archive. |

### Compatibility

| Method | Path | Purpose |
|---|---|---|
| GET | `/api/v1/memories/list` | Compatibility alias for canonical `/api/v1/memories`. |
| GET | `/api/v1/memory?id=&include_suppressed=` | Legacy compact single-memory endpoint. |

## Capabilities

`GET /api/v1/capabilities` returns:

```json
{
  "version": "0.6.6",
  "schema_version": 70,
  "api_version": 1,
  "features": {
    "health": true,
    "status": true,
    "stats": true,
    "search": true,
    "search_explain": true,
    "memory_list": true,
    "memory_detail": true,
    "save_memory": true,
    "memory_archive": true,
    "memory_restore": true,
    "memory_delete": false,
    "candidate_rows": true,
    "candidate_filters": true,
    "candidate_review": true,
    "candidate_detail": true,
    "candidate_evidence": true,
    "candidate_review_safe": true,
    "observations": true,
    "sessions": true,
    "workstreams": true,
    "events": true,
    "tasks": true,
    "graph": true,
    "user_recall": true,
    "user_recall_usage_policy": true
  },
  "endpoints": {
    "health": "/api/v1/health",
    "status": "/api/v1/status",
    "stats": "/api/v1/stats",
    "search": "/api/v1/search",
    "search_explain": "/api/v1/search?explain=true",
    "memory_list": "/api/v1/memories",
    "memory_detail": "/api/v1/memories/{id}",
    "save_memory": "/api/v1/memories",
    "memory_archive": "/api/v1/memories/{id}/archive",
    "memory_restore": "/api/v1/memories/{id}/restore",
    "candidate_rows": "/api/v1/candidates",
    "candidate_blocked": "/api/v1/candidates/blocked",
    "candidate_review": "/api/v1/candidates/{id}/approve",
    "candidate_detail": "/api/v1/candidates/{id}",
    "candidate_evidence": "/api/v1/candidates/{id}",
    "candidate_review_safe_approve": "/api/v1/candidates/{id}/review/approve",
    "candidate_review_safe_reject": "/api/v1/candidates/{id}/review/reject",
    "candidate_review_safe_edit": "/api/v1/candidates/{id}/review/edit",
    "observations_list": "/api/v1/observations",
    "observations_detail": "/api/v1/observations/{id}",
    "sessions_list": "/api/v1/sessions",
    "sessions_detail": "/api/v1/sessions/{id}",
    "workstreams_list": "/api/v1/workstreams",
    "workstreams_detail": "/api/v1/workstreams/{id}",
    "events_list": "/api/v1/events",
    "events_detail": "/api/v1/events/{id}",
    "tasks_list": "/api/v1/tasks",
    "tasks_detail": "/api/v1/tasks/{id}",
    "graph": "/api/v1/graph",
    "user_recall": "/api/v1/user/recall"
  }
}
```

Feature flags are the client gate. A web client should not infer support from
package metadata alone. Clients that render user-recall usage-policy guidance
must require `features.user_recall_usage_policy`, not only
`features.user_recall`, because remem `0.5.114` through `0.5.122` expose user
recall without returning `usage_policy`.

## Response Contracts

`GET /api/v1/health` is the fast liveness endpoint. It requires the same bearer
token as other `/api/v1/*` routes, does not run aggregate system stats, and
returns:

```json
{
  "ok": true,
  "version": "0.5.123",
  "api_version": 1,
  "schema_version": 52
}
```

`GET /api/v1/status` preserves its existing counters and adds machine-readable
cache metadata. The default cache TTL is 2 seconds:

```json
{
  "version": "0.5.114",
  "memories": 10,
  "observations": 20,
  "cache": {
    "hit": false,
    "stale": false,
    "generated_at_epoch": 1781940000,
    "ttl_secs": 2
  }
}
```

`GET /api/v1/status?refresh=true` bypasses the cache and recomputes status. If
refresh fails but a bounded stale payload is still available, the response is
HTTP 200 with `cache.stale=true` and a `warnings` array. Without an acceptable
stale payload, status returns the existing structured error response.

List endpoints return:

```json
{
  "data": [],
  "meta": {
    "count": 0,
    "total": 0,
    "limit": 50,
    "offset": 0,
    "has_more": false,
    "next_offset": null
  }
}
```

`GET /api/v1/search` keeps its existing search-specific `meta` shape and may
also include `multi_hop`, `raw_hits`, `raw_hits_error`, and `explain`.
Default search, memory browse, graph, and direct memory detail reads exclude
policy-suppressed memories. Search also disables raw-archive fallback when
active suppressions are present, so raw text cannot bypass a "do not mention/use
by default" policy. Pass `include_suppressed=true` only for explicit audit views
that need to inspect suppressed evidence.

`GET /api/v1/graph` returns only DB-backed data:

```json
{
  "nodes": [],
  "edges": []
}
```

Empty graph or candidate tables return empty arrays, not synthesized rows.

`GET /api/v1/candidates` defaults to `status=pending_review` and accepts the
same review filters used by batch review commands: `project`, `type`,
`block_reason`, `topic_key`, `contains`, `min_confidence`, and
`older_than_days`. `contains` matches candidate text or topic key.
`older_than_days` selects rows created on or before the computed cutoff.
`limit` is clamped to `1..=100`.

`GET /api/v1/candidates/blocked?project=` returns block-reason aggregates:

```json
{
  "data": [
    {
      "reason": "unsupported_type",
      "pending": 12,
      "example_ids": [101, 102, 103]
    }
  ],
  "meta": {
    "count": 1,
    "total": 1,
    "limit": 1,
    "offset": 0,
    "has_more": false,
    "next_offset": null
  }
}
```

Candidate review responses are explicit:

```json
{
  "candidate_id": 1,
  "status": "approved",
  "memory_id": 123
}
```

`POST /api/v1/candidates/{id}/approve` normally accepts an empty body. For a
quarantined memory candidate, clients must send the matched pattern id:

```json
{
  "acknowledge_pattern": "override_previous_instructions"
}
```

`POST /api/v1/candidates/{id}/edit` accepts any changed subset of:

```json
{
  "scope": "project",
  "memory_type": "decision",
  "topic_key": "native-api-contract",
  "text": "edited memory text"
}
```

All normal control-flow errors use:

```json
{
  "error": {
    "code": "not_found",
    "message": "Memory not found"
  }
}
```

Candidate review errors include `not_found`, `candidate_not_pending`,
`candidate_quarantined`, `candidate_acknowledgement_invalid`,
`candidate_edit_invalid`, and `candidate_review_failed`.

### GH-880 safe candidate review

`GET /api/v1/candidates/{id}` returns `data`, `evidence`, and `decision`.
Candidate data includes the current integer `version`. Evidence contains only
allowlisted provenance and derived summaries; it never returns captured-event
`content_text`, blobs, environment payloads, or raw transcripts. If evidence is
missing, cross-project, suppressed, unsafe, or otherwise unverifiable,
`decision.can_review=false` and `blocked_reasons` contains stable codes. The
server does not fall back to raw evidence.

The three `/review/*` routes require:

```json
{
  "reason": "reviewed against source evidence",
  "expected_version": 4,
  "idempotency_key": "client-generated-stable-key"
}
```

Approve may additionally carry `acknowledge_pattern`; edit may carry the
existing editable candidate fields. `reason` is trimmed and must contain
1–1024 UTF-8 bytes. The idempotency key is trimmed and must contain 1–128 ASCII
bytes from `[A-Za-z0-9._~-]`; its plaintext never enters DB, audit, log, or
response. A successful response is an audit envelope:

```json
{
  "response_schema_version": 1,
  "operation_id": "op_<sha256>",
  "audit_id": 42,
  "candidate_id": 7,
  "memory_id": 12,
  "action": "approve",
  "before_status": "pending_review",
  "after_status": "approved",
  "version": 5,
  "occurred_at_epoch": 1784340000,
  "replayed": false
}
```

Same key and normalized payload replays that envelope with only
`replayed=true`; a different payload returns `409 idempotency_conflict` before
current candidate state is inspected. Stable safe-review errors include
`version_conflict`, `candidate_not_reviewable`, `evidence_blocked`,
`idempotency_key_invalid`, `reason_invalid`, `idempotency_conflict`, and
`idempotency_schema_unsupported`.

### GH-880 safe read resources

Observations, sessions, workstreams, events, and tasks are separate capability
bundles. Each list response has:

```json
{
  "data": [],
  "page_size": 50,
  "next_cursor": null
}
```

`page_size` defaults to 50, clamps parsed integers to 1–100, and rejects
malformed or overflowing values with `page_size_invalid`. `next_cursor` is an
opaque, versioned keyset cursor bound to the resource kind and effective
filters. A malformed, cross-resource, or filter-mismatched cursor returns
`cursor_invalid`. Suppression-aware bounded scans may return a partial or empty
page with a non-null advancing cursor; clients must continue until
`next_cursor=null`.

Detail responses contain a single safe row under `data`; missing or suppressed
rows return 404. Projection-policy failures return structured 5xx responses,
not empty data. Events and observations exclude raw `content_text` and blobs;
tasks expose only classified error state rather than `last_error` text; related
resources are bounded safe references and are not recursively expanded. These
routes intentionally have no `include_suppressed` override. Legacy
`/api/v1/search.raw_hits[].preview` retains its pre-GH-880 compatibility
contract and is not a safe-resource projection.

### GH-825 Cursor session capture health

When GH-825 ships, `GET /api/v1/capabilities.features` adds
`session_capture_health: true`; clients must gate the field below on that
capability and on a source/release version containing GH-825. The existing
`sessions` flag alone does not prove capture-health support.

`GET /api/v1/sessions/{id}` then adds one nullable safe field:

```json
{
  "data": {
    "capture_health": {
      "fidelity": "full",
      "status": "completed",
      "reason_code": null,
      "stop_key": "<redacted-bounded-key>"
    }
  }
}
```

`capture_health` is `null` for non-Cursor sessions or when no authoritative
Cursor outcome exists. For Cursor outcomes, `fidelity` is exactly
`full|degraded|blank`; `status` is drawn only from the GH-823-approved Stop
set; `reason_code` is null for full and a stable non-content code for degraded
or blank; `stop_key` is a bounded redacted locator, never the raw transcript
path or payload. The handler first selects the current source Stop by its
immutable capture order
`(captured_events.created_at_epoch, captured_events.id)`, then resolves
`full > degraded > blank` only within that Stop, matching CLI and context.
Outcome insertion time or replay cannot advance the selected Stop, and an
earlier full cannot mask a later degraded/blank result. It follows a summary
reference only for full/degraded outcomes that actually carry one. A blank outcome returns the
visible `capture_health` object with `reason_code: "no_usable_evidence"` and
does not dereference or synthesize a summary. Old blank/degraded outcomes
remain auditable but are never returned beside a higher-priority result.
Malformed outcome JSON or DB read failure returns the existing structured 5xx
error rather than `capture_health: null`.

### GH-880 recoverable memory governance

Memory list and detail responses add the current integer `version` while
preserving the canonical no-status list behavior. remem-web should request
`status=active` for its default view and `status=archived` for restore/audit
views. Default search continues excluding archived memories.

Archive and restore use the same `reason`, `expected_version`, and
`idempotency_key` validation as safe candidate review. Archive permits only
`active -> archived`. Restore permits only an archived row whose current Web
archive marker matches the exact successful archive ledger and audit; a
historical ledger alone never authorizes restore. The success envelope uses
the same schema as safe candidate review with `memory_id` in place of
`candidate_id` and returns the final version, which can directly drive the next
restore/archive request.

Ledger replay/conflict is checked before current memory state. Missing,
deleted, non-Web archived, or provenance-mismatched restore returns
`404 memory_not_recoverable`; stale versions return `409 version_conflict`;
non-active archive returns `409 memory_not_archivable`. State, marker, audit,
and replay ledger commit atomically. Permanent Web delete is not implemented:
`features.memory_delete=false`, there is no endpoint key, and no DELETE route
is registered.

`POST /api/v1/user/recall` accepts:

```json
{
  "query": "current task",
  "project": "/repo/path",
  "cwd": "/repo/path",
  "task_intent": "optional task intent",
  "current_files": ["src/lib.rs"],
  "host": "codex",
  "owner_scope": "user",
  "owner_key": null,
  "state_keys": ["current-memory-preferences"],
  "include_sensitive": false,
  "include_suppressed": false,
  "limit": 6,
  "budget_chars": 4000
}
```

`query` and either `project` or `cwd` are required. Empty recall is explicit and
does not synthesize a generic profile:

```json
{
  "query": "current task",
  "project": "/repo/path",
  "task_intent": null,
  "host": "codex",
  "empty": true,
  "context": "",
  "usage_policy": null,
  "included": [],
  "dropped": [],
  "diagnostics": {
    "requested_limit": 6,
    "budget_chars": 4000,
    "used_chars": 0,
    "candidate_counts": {
      "summaries": 0,
      "claims": 0,
      "memories": 0,
      "current_state": 0,
      "workstreams": 0,
      "sessions": 0,
      "dropped": 0
    }
  }
}
```

Non-empty recall includes source-attributed context plus a separate
`usage_policy` string. Clients must not concatenate `usage_policy` into
`context` before enforcing the context budget:

```json
{
  "empty": false,
  "context": "- [user_claim:1] preference:status_reports: Prefers concise status reports.",
  "usage_policy": "Use user context only when it materially improves the current answer. Prefer invisible adaptation over explicit memory narration. Limit explicit memory prose mentions to 0-1 per response; required final citation lines are exempt. Do not say \"I remember you said\" or \"from previous conversations\" unless the user is discussing memory, provenance, or correction. Do not infer profile facts beyond the cited items. If no user context applies, do not invent a profile.",
  "included": [
    {
      "source_type": "user_claim",
      "source_id": 1,
      "title": "preference:status_reports",
      "text": "Prefers concise status reports.",
      "reason_codes": [
        "active_user_claim",
        "query_match",
        "owner:user:default"
      ],
      "source_refs": [
        {
          "kind": "manual_cli",
          "ts": 1782291600
        }
      ]
    }
  ],
  "dropped": [
    {
      "source_type": "user_claim",
      "source_id": 2,
      "label": "Sensitive identity detail",
      "reason_code": "sensitivity:sensitive"
    }
  ],
  "diagnostics": {
    "requested_limit": 6,
    "budget_chars": 4000,
    "used_chars": 64,
    "candidate_counts": {
      "summaries": 0,
      "claims": 2,
      "memories": 0,
      "current_state": 0,
      "workstreams": 0,
      "sessions": 0,
      "dropped": 1
    }
  }
}
```

`included[].reason_codes` are emitted as machine strings from the recall source
collector. User-claim rows include `active_user_claim`, `query_match`, and an
`owner:<scope>:<key>` marker. `dropped[].reason_code` is also an exact machine
string; common values include `status:<status>`, `sensitivity:<classification>`,
`not_yet_valid`, `expired`, `policy_suppressed`, `not_relevant`, and
`budget_exceeded`.

## Security And Side Effects

- API binds only to `127.0.0.1`.
- Every route requires `Authorization: Bearer <token>` from the data-dir
  `.api-token` file.
- Queries use parameterized SQL placeholders.
- `GET /api/v1/health`, `/status`, `/capabilities`, `/stats`, `/search`,
  `/memories`, `/candidates`, `/candidates/blocked`, and `/graph` do not
  modify durable memory content.
- `/health` is for cheap liveness. `/capabilities` is for feature detection.
  `/status` is for dashboard counters and should not be polled more frequently
  than its returned `cache.ttl_secs` unless the user explicitly requests
  `/status?refresh=true`.
- `GET /api/v1/memories/{id}` and legacy `GET /api/v1/memory?id=` update
  memory access telemetry on successful detail reads.
- Candidate review endpoints are transactional. If promotion fails, the
  candidate must remain pending and no memory should be partially created.

## Release Gate

Release notes for web API changes must identify:

- the minimum `remem` version needed by remem-web;
- which `/api/v1/capabilities.features` are available;
- whether `/api/v1/status` responses include cache metadata;
- compatibility guidance for `/api/v1/memory?id=` and `/api/v1/memories/list`;
- whether candidates are list-only or include review actions.
- whether candidate review filters and `/api/v1/candidates/blocked` are
  available.

For the first complete native web API surface, the release target is
`remem 0.5.109`. Do not document it as available to installed-binary users
until the `v0.5.109` tag and GitHub Release exist.

For fast health and cached status, the release target is `remem 0.5.112`.
Do not document those as installed-binary behavior until the corresponding tag
and GitHub Release exist.

For suppression audit opt-in, the release target is `remem 0.5.113`. Default
read surfaces must continue excluding suppressed memories unless
`include_suppressed=true` is explicitly requested by an audit surface.

For task-aware user recall, the release target is `remem 0.5.114`. Clients
must gate the UI on `capabilities.features.user_recall` and call
`POST /api/v1/user/recall` instead of widening SessionStart context.

For user recall usage-policy guidance, the release target is `remem 0.5.123`.
Clients must gate usage-policy guidance on
`capabilities.features.user_recall_usage_policy`. When that flag is present,
clients should treat `usage_policy` as response metadata for non-empty recall
results and should not count it against the recalled context budget.

For candidate review queue throughput, the release target is `remem 0.5.162`.
Clients that render review dashboard filters must require
`capabilities.features.candidate_filters`, not only `candidate_rows`, because
older binaries expose candidate rows while ignoring unknown filter parameters.
Clients that render blocked-reason aggregates must also require
`capabilities.endpoints.candidate_blocked`.

For the GH-880 safe console contract, the release target is `remem 0.6.6`.
Clients must require each feature flag together with its exact endpoint-map
bundle; they must not infer paths. Candidate detail/evidence requires both
detail keys plus all three safe-review action keys before enabling review.
Each safe read resource requires its own list/detail pair. Archive and restore
are gated independently. `memory_delete` remains false. Source metadata and an
`unreleased` runtime manifest are not installed-binary evidence: remem-web must
wait for the `v0.6.6` tag, published release assets, and the minimum-version
gate before enabling these views.

## Smoke Test

Run:

```bash
scripts/smoke_native_web_api.sh
```

The smoke starts a local built `remem api` process in an isolated
`REMEM_DATA_DIR`, reads the generated API token, and verifies every advertised
GH-880 list/detail/action template under bearer-token auth. It covers safe
candidate review, all five read-resource bundles, archive/restore, delete
absence, and the legacy search raw-hit compatibility shape. It must not print
or leak the token or raw idempotency keys.
