# SPEC: remem-web local REST API

Status: current contract. Refs #568.

The native web API is the localhost, bearer-token boundary used by remem-web
and Apps SDK clients. Clients must not read the SQLCipher database directly or
invent mock graph/candidate data when the native binary lacks an endpoint.

The complete web read-model surface is implemented in source version
`0.5.109`. Fast health checks and cached status metadata are implemented in
source version `0.5.112`. Suppression audit opt-in is implemented in source
version `0.5.113`. Task-aware user recall is implemented in source version
`0.5.114`. remem-web should require a published release with the specific
capability it needs before directing installed-binary users to that surface.
Clients should call `GET /api/v1/capabilities` before enabling optional UI
features.

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
| GET | `/api/v1/candidates?project=&status=&limit=&offset=` | Compact memory-candidate list. |
| POST | `/api/v1/candidates/{id}/approve` | Approve a pending candidate. |
| POST | `/api/v1/candidates/{id}/reject` | Reject a pending candidate; persisted status is `discarded`. |
| POST | `/api/v1/candidates/{id}/edit` | Edit and approve a pending candidate. |
| GET | `/api/v1/graph?project=&limit=&include_suppressed=` | DB-backed entity graph. |

### Compatibility

| Method | Path | Purpose |
|---|---|---|
| GET | `/api/v1/memories/list` | Compatibility alias for canonical `/api/v1/memories`. |
| GET | `/api/v1/memory?id=&include_suppressed=` | Legacy compact single-memory endpoint. |

## Capabilities

`GET /api/v1/capabilities` returns:

```json
{
  "version": "0.5.114",
  "schema_version": 51,
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
    "candidate_rows": true,
    "candidate_review": true,
    "graph": true,
    "user_recall": true
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
    "candidate_rows": "/api/v1/candidates",
    "candidate_review": "/api/v1/candidates/{id}/approve",
    "graph": "/api/v1/graph",
    "user_recall": "/api/v1/user/recall"
  }
}
```

Feature flags are the client gate. A web client should not infer support from
package metadata alone.

## Response Contracts

`GET /api/v1/health` is the fast liveness endpoint. It requires the same bearer
token as other `/api/v1/*` routes, does not run aggregate system stats, and
returns:

```json
{
  "ok": true,
  "version": "0.5.114",
  "api_version": 1,
  "schema_version": 51
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

Candidate review responses are explicit:

```json
{
  "candidate_id": 1,
  "status": "approved",
  "memory_id": 123
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
`candidate_edit_invalid`, and `candidate_review_failed`.

## Security And Side Effects

- API binds only to `127.0.0.1`.
- Every route requires `Authorization: Bearer <token>` from the data-dir
  `.api-token` file.
- Queries use parameterized SQL placeholders.
- `GET /api/v1/health`, `/status`, `/capabilities`, `/stats`, `/search`,
  `/memories`, `/candidates`, and `/graph` do not modify durable memory
  content.
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

## Smoke Test

Run:

```bash
scripts/smoke_native_web_api.sh
```

The smoke starts a local built `remem api` process in an isolated
`REMEM_DATA_DIR`, reads the generated API token, and verifies the documented
read endpoints under bearer-token auth. It must not print or leak the token.
