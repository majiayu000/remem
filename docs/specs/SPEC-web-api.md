# SPEC: remem-web local REST API

Status: current contract. Refs #568.

The native web API is the localhost, bearer-token boundary used by remem-web
and Apps SDK clients. Clients must not read the SQLCipher database directly or
invent mock graph/candidate data when the native binary lacks an endpoint.

The complete web read-model surface is implemented in source version
`0.5.109`. remem-web should require a published `remem >= 0.5.109` release
before directing installed-binary users to the full API surface. Clients should
call `GET /api/v1/capabilities` before enabling optional UI features.

## Endpoint Groups

### Stable Core

| Method | Path | Purpose |
|---|---|---|
| GET | `/api/v1/status` | Operational health, queue state, and counters. |
| GET | `/api/v1/capabilities` | Native feature and endpoint discovery. |
| GET | `/api/v1/search?query=&project=&type=&limit=&offset=&branch=&include_stale=&multi_hop=&explain=` | Search memories with optional explain. |
| GET | `/api/v1/memory?id=` | Legacy compact single-memory endpoint. |
| GET | `/api/v1/memories?project=&type=&scope=&status=&branch=&q=&limit=&offset=` | Canonical browse endpoint. |
| GET | `/api/v1/memories/{id}` | Rich detail with entities and memory edges. |
| POST | `/api/v1/memories` | Explicit durable memory save. |

### Web Read Model

| Method | Path | Purpose |
|---|---|---|
| GET | `/api/v1/stats` | Product stats for local dashboards. |
| GET | `/api/v1/candidates?project=&status=&limit=&offset=` | Compact memory-candidate list. |
| POST | `/api/v1/candidates/{id}/approve` | Approve a pending candidate. |
| POST | `/api/v1/candidates/{id}/reject` | Reject a pending candidate; persisted status is `discarded`. |
| POST | `/api/v1/candidates/{id}/edit` | Edit and approve a pending candidate. |
| GET | `/api/v1/graph?project=&limit=` | DB-backed entity graph. |

### Compatibility

| Method | Path | Purpose |
|---|---|---|
| GET | `/api/v1/memories/list` | Compatibility alias for canonical `/api/v1/memories`. |
| GET | `/api/v1/memory?id=` | Legacy compact single-memory endpoint. |

## Capabilities

`GET /api/v1/capabilities` returns:

```json
{
  "version": "0.5.109",
  "schema_version": 48,
  "api_version": 1,
  "features": {
    "status": true,
    "stats": true,
    "search": true,
    "search_explain": true,
    "memory_list": true,
    "memory_detail": true,
    "save_memory": true,
    "candidate_rows": true,
    "candidate_review": true,
    "graph": true
  },
  "endpoints": {
    "status": "/api/v1/status",
    "stats": "/api/v1/stats",
    "search": "/api/v1/search",
    "search_explain": "/api/v1/search?explain=true",
    "memory_list": "/api/v1/memories",
    "memory_detail": "/api/v1/memories/{id}",
    "save_memory": "/api/v1/memories",
    "candidate_rows": "/api/v1/candidates",
    "candidate_review": "/api/v1/candidates/{id}/approve",
    "graph": "/api/v1/graph"
  }
}
```

Feature flags are the client gate. A web client should not infer support from
package metadata alone.

## Response Contracts

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
- `GET /api/v1/status`, `/capabilities`, `/stats`, `/search`, `/memories`,
  `/candidates`, and `/graph` do not modify durable memory content.
- `GET /api/v1/memories/{id}` and legacy `GET /api/v1/memory?id=` update
  memory access telemetry on successful detail reads.
- Candidate review endpoints are transactional. If promotion fails, the
  candidate must remain pending and no memory should be partially created.

## Release Gate

Release notes for web API changes must identify:

- the minimum `remem` version needed by remem-web;
- which `/api/v1/capabilities.features` are available;
- compatibility guidance for `/api/v1/memory?id=` and `/api/v1/memories/list`;
- whether candidates are list-only or include review actions.

For the first complete native web API surface, the release target is
`remem 0.5.109`. Do not document it as available to installed-binary users
until the `v0.5.109` tag and GitHub Release exist.

## Smoke Test

Run:

```bash
scripts/smoke_native_web_api.sh
```

The smoke starts a local built `remem api` process in an isolated
`REMEM_DATA_DIR`, reads the generated API token, and verifies the documented
read endpoints under bearer-token auth. It must not print or leak the token.
