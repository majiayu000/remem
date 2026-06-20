# Fast Health and Cached Status API Product Spec

Status: Current contract
Date: 2026-06-20

Tracking:
- Fast health and cached status API: #588

## Problem

`GET /api/v1/status` currently serves two jobs for local web clients:

- liveness: is the authenticated local API process reachable?
- diagnostics: what are the current aggregate memory, capture, candidate, and
  worker counters?

Those jobs have different latency and freshness requirements. A liveness probe
should be cheap and low variance. A diagnostics endpoint can do heavier
database work because users read it less frequently.

Local measurements on an installed `remem 0.5.65` runtime with an approximately
1.1 GB SQLCipher SQLite database showed:

| Probe | Result |
| --- | --- |
| `GET /api/v1/status`, first request | about 496 ms |
| `GET /api/v1/status`, warm requests | about 214-236 ms |
| TCP connect to `127.0.0.1:5567` | about 0.3 ms |
| `remem status --json`, warm CLI runs | about 0.53-0.54 s |
| `GET /api/v1/search?query=remem&limit=1`, warm requests | about 108-111 ms |
| `GET /api/v1/search?query=*&limit=1`, warm requests | about 285-294 ms |

The HTTP path is not the bottleneck. The likely cost is server-side database
open plus the shared `query_system_stats` aggregate queries. Web clients should
not have to poll that aggregate path just to know whether remem is alive.

## Goals

1. Add a fast, authenticated HTTP liveness endpoint for local clients.
2. Keep `/api/v1/status` useful for system status without encouraging frequent
   heavyweight polling.
3. Preserve the existing `/api/v1/status` fields and add cache metadata
   compatibly.
4. Give remem-web and other native clients a clear polling policy.
5. Add implementation tests and smoke coverage so future regressions are
   visible.

## Non-Goals

- Do not remove or rename `/api/v1/status`.
- Do not change memory search ranking, candidate review semantics, graph
  extraction, capture, or worker behavior.
- Do not expose SQLCipher database access to web clients.
- Do not add a metrics daemon, Prometheus server, or always-on background
  service.
- Do not silently hide database errors from diagnostic endpoints.

## Product Model

Split the API into three explicit tiers.

| Tier | Endpoint or command | Client intent |
| --- | --- | --- |
| Fast health | `GET /api/v1/health` | Cheap authenticated liveness and API readiness. |
| Cached status | `GET /api/v1/status` | Dashboard counters with short bounded staleness. |
| Heavy diagnostics | `GET /api/v1/status?refresh=true`, `GET /api/v1/stats`, `remem status --json`, `remem doctor` | Fresh operational diagnostics. |

### Fast Health

`GET /api/v1/health` should answer whether the local authenticated API process
is alive and able to serve v1 clients. It must require the same bearer token as
the rest of the API and must not return token values, filesystem paths, or other
secrets.

The endpoint should avoid aggregate database queries. If schema information
requires database access, the implementation should do only the minimum metadata
read required, not status aggregation.

### Cached Status

`GET /api/v1/status` remains the dashboard status endpoint. Repeated requests
within a short TTL should return cached status with explicit cache metadata.

Recommended defaults:

| Field | Default |
| --- | --- |
| Cache TTL | 2 seconds |
| Max stale on refresh failure | 10 seconds |
| Cache scope | Process-local, per running `remem api` |
| Force refresh | `GET /api/v1/status?refresh=true` |

Cached status may be a few seconds stale, but it must be marked as cached and
must never pretend to be a fresh diagnostic result.

### Heavy Diagnostics

Fresh diagnostic commands remain available for users and support workflows that
need current counters. Web clients should use:

- `/api/v1/health` for liveness;
- `/api/v1/capabilities` for feature detection;
- `/api/v1/status` for dashboard counters no more frequently than the cache TTL;
- `/api/v1/status?refresh=true` only for explicit refresh actions.

## Success Metrics

| Metric | Current | Target | Measurement |
| --- | --- | --- | --- |
| `/api/v1/health` warm p95 latency | Not available | < 25 ms | Local smoke loop against `remem api` |
| `/api/v1/status` warm cache-hit p95 latency | about 214-236 ms observed | < 50 ms | Local smoke loop with repeated requests |
| `/api/v1/status?refresh=true` p95 latency | about 214-236 ms observed | No regression over current | Local smoke loop |
| Status cache correctness | Not available | Cached payload age <= configured TTL unless marked stale | Handler tests |
| Auth behavior | Token required today | Same token requirement for health/status | Handler tests |
| Web polling load | Sidebar can call `/status` on navigation | Web uses `/health` or cached status; no sub-TTL polling | remem-web integration review |

## Alternatives Considered

### Option A: Add `/health` and cache `/status` (Recommended)

This gives clients a true fast liveness endpoint, reduces aggregate database
pressure, preserves `/status` compatibility, and keeps fresh diagnostics
available through `refresh=true`.

Tradeoff: the API grows by one endpoint and status can be briefly stale by
default.

### Option B: Only cache `/status`

This is smaller, but clients still lack a clear heartbeat endpoint and the first
status request remains heavy. Rejected as incomplete.

### Option C: Add status levels

Examples:

```http
GET /api/v1/status?level=basic
GET /api/v1/status?level=full
```

This avoids a new top-level endpoint, but query flags are easier to misuse than
separate routes and existing clients still call the old heavy default. Rejected
for v1.

### Option D: Persist a status snapshot table

This can make reads fast, but it adds write-side drift risk across worker,
capture, candidate review, and migration paths. Defer until profiling proves an
in-process cache is not enough.

## Risks And Mitigations

| Risk | Severity | Mitigation |
| --- | --- | --- |
| Cached status hides a fresh database failure | Medium | Cache only successful status payloads, mark stale responses, bound max stale, and keep `refresh=true`. |
| Cache introduces shared mutable state bugs | Medium | Keep cache inside API state and cover hit, miss, refresh, stale, and error paths with handler tests. |
| `/health` becomes another heavy endpoint | Medium | Contract forbids aggregate queries; implementation review should check it does not call `query_system_stats`. |
| Clients misuse `/health` as feature detection | Low | Document `/capabilities` as the feature detection endpoint. |
| Performance thresholds are flaky on slow machines | Medium | Use deterministic unit tests for cache behavior and use smoke latency as a regression signal, not the only gate. |

## Acceptance Criteria

- `GET /api/v1/health` exists, is authenticated, and returns stable JSON.
- `/api/v1/capabilities` advertises the health endpoint.
- `/api/v1/status` preserves existing fields and adds optional cache metadata.
- `/api/v1/status?refresh=true` recomputes status.
- Repeated `/api/v1/status` requests within the TTL are served from cache.
- Handler tests cover auth, cache hit, refresh, stale fallback, and structured
  error behavior.
- README documents the liveness/status/stats distinction.
- A smoke script or documented command verifies health and repeated status calls
  under bearer-token auth.
- remem-web can stop treating `/status` as a heartbeat and can use `/health` or
  `/capabilities` instead.

## Open Questions

1. Should `schema_version` be required in `/health` if reading it opens the
   database? Recommendation: allow `schema_version: null` or omit it if that is
   necessary to keep health fast.
2. Should cache TTL be configurable? Recommendation: use a fixed 2 seconds first
   and add configuration only after real users need tuning.
3. Should stale status responses use HTTP 200 with `cache.stale=true`, or a
   warning status code? Recommendation: use HTTP 200 only inside the short
   max-stale window because the payload is explicitly marked stale.
4. Should CLI `remem status --json` use the same cache? Recommendation: no. CLI
   status should stay fresh unless a future `--cached` flag is added.
