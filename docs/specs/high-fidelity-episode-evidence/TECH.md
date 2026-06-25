# High-Fidelity Episode Evidence Technical Spec

Status: Current contract
Issue: #626

## Current Implementation Truth

The repository already has evidence and provenance paths that high-fidelity
mode must reuse:

- `captured_events` stores normalized hook events, compact previews, large
  capture blobs, retention class, session identity, project identity, and
  `reference_time_epoch`.
- `raw_messages` stores raw user/assistant transcript turns for raw recall.
- `compressed_observation_sources` links compressed observations to their
  source observations.
- `memories`, `memory_facts`, `memory_edges`, `memory_state_keys`, and
  `memory_operation_log` own durable curated memory, temporal truth, lifecycle,
  current slots, and audit.
- `graph_edges` can cite trusted `source_event_ids`.
- `context_injections` and `context_injection_items` own output-level and
  item-level context injection audit.
- `memory_citation_events` and `memory_usage_events` own Stop-time citation and
  usage feedback.
- `search_raw`, MCP `search_raw`, `get_observations`, `current_state`,
  timeline, REST search/detail, and benchmark artifacts already expose parts of
  this evidence.

Implementation must extend these paths. It must not add a second memory store,
new ungoverned transcript archive, or a parallel retrieval stack.

## Data Model Direction

The preferred implementation is a small metadata layer over existing source
tables.

Proposed table:

```sql
CREATE TABLE high_fidelity_source_slices (
    id INTEGER PRIMARY KEY,
    project TEXT NOT NULL,
    branch TEXT,
    host TEXT,
    session_id TEXT,
    scope TEXT NOT NULL CHECK (scope IN ('benchmark', 'project', 'session')),
    source_kind TEXT NOT NULL CHECK (
        source_kind IN (
            'captured_event',
            'raw_message',
            'compressed_observation_source',
            'fixture_episode'
        )
    ),
    source_ids_json TEXT NOT NULL CHECK (json_valid(source_ids_json) = 1),
    reference_time_epoch INTEGER,
    retained_at_epoch INTEGER NOT NULL,
    retention_reason TEXT NOT NULL,
    policy_status TEXT NOT NULL CHECK (
        policy_status IN ('retained', 'redacted', 'suppressed', 'deleted', 'blocked')
    ),
    byte_count INTEGER NOT NULL DEFAULT 0,
    purge_after_epoch INTEGER,
    content_hash TEXT,
    metadata_json TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(metadata_json) = 1)
);
```

Proposed link table:

```sql
CREATE TABLE high_fidelity_source_links (
    id INTEGER PRIMARY KEY,
    source_slice_id INTEGER NOT NULL
        REFERENCES high_fidelity_source_slices(id) ON DELETE CASCADE,
    target_kind TEXT NOT NULL CHECK (
        target_kind IN (
            'memory',
            'memory_fact',
            'observation',
            'memory_candidate',
            'graph_edge',
            'context_injection_item',
            'citation_event',
            'usage_event',
            'benchmark_run'
        )
    ),
    target_id INTEGER,
    target_key TEXT,
    relation TEXT NOT NULL CHECK (
        relation IN ('supports', 'contradicts', 'was_injected', 'was_cited', 'was_used', 'missing_gold')
    ),
    created_at_epoch INTEGER NOT NULL
);
```

These tables hold metadata and links. Raw text remains in existing source
tables or fixture files. Public artifacts should normally export hashes, ids,
counts, and redacted excerpts, not private content.

## Activation And Runtime Gating

High-fidelity mode is off unless an implementation explicitly passes an
activation policy into the capture, extraction, benchmark, or debug path.

Suggested policy shape:

```rust
enum HighFidelityEvidenceMode {
    Off,
    Benchmark { run_id: String, byte_cap: u64 },
    Project { project: String, byte_cap: u64, purge_after_epoch: i64 },
    Session { session_id: String, byte_cap: u64, purge_after_epoch: i64 },
}
```

Activation rules:

- hooks default to `Off`;
- benchmark commands may enable `Benchmark` for temporary data dirs;
- doctor/debug commands may inspect mode state but must not enable it by
  reading config alone;
- any persistent project/session activation must be explicit and reviewable.

## Preservation Flow

1. Capture writes normal `captured_events` and raw archive rows.
2. Extraction, summary, compression, promotion, graph, current-state, and
   injection code continue to cite existing source event ids and memory ids.
3. When high-fidelity mode is active, a preservation layer records source slice
   metadata for the relevant source ids.
4. The preservation layer records links from slices to memories, facts,
   candidates, injection items, citations, usage events, or benchmark runs.
5. Search/detail/current-state/benchmark reporting reads slice metadata only
   after normal policy filters have run.

No source slice may be marked `retained` unless the underlying source text has
passed existing redaction and non-retention checks. If policy blocks the slice,
record `policy_status='blocked'` with ids and counts where safe.

## Policy And Governance

Suppression/deletion semantics must be propagated:

- memory suppression hides linked slices from default read paths;
- user-context suppression hides linked slices from profile, recall, injection,
  and public artifact output;
- destructive governance marks linked slices `deleted` or removes metadata when
  the underlying source is deleted;
- purge removes expired slices and their links in the same transaction;
- public artifact export refuses any slice with `policy_status` other than
  `retained` or `redacted`.

Failures must not degrade silently. If a benchmark requires a source slice and
policy blocks or purges it, the report records `missing_source_evidence` or
`policy_abstained`.

## Surface Contracts

### CLI And MCP

`search_raw` remains a raw recall surface. Future high-fidelity fields should
be compact and optional:

```json
{
  "high_fidelity_source": {
    "mode": "benchmark",
    "slice_id": 12,
    "policy_status": "retained",
    "reference_time_epoch": 1760000000
  }
}
```

`get_observations` and `current_state` may expose linked slice ids and statuses
beside existing compressed-source and staleness metadata. They must not dump
full raw text unless the caller explicitly asks for raw source expansion and
policy allows it.

### REST

REST search/detail may expose the same compact metadata in authenticated local
responses. Browser/app UI must treat raw source expansion as an explicit user
action.

### Benchmark Artifacts

Coding and memory benchmark runs should include:

```json
{
  "high_fidelity_mode": "benchmark",
  "source_evidence": {
    "required_source_ids": ["episode-a:e17"],
    "preserved_source_ids": ["episode-a:e17"],
    "missing_source_ids": [],
    "policy_blocked_source_ids": [],
    "retrieved_source_ids": ["episode-a:e17"],
    "cited_source_ids": ["episode-a:e17"]
  }
}
```

The artifact verifier must reject missing mode metadata for suites that require
high-fidelity evidence.

## Storage Bounds

Implementation must include:

- byte-count accounting per slice and per scope;
- cap checks before export and before committing benchmark artifacts;
- `purge_after_epoch` and explicit cleanup;
- doctor/status warnings for over-budget retained slices, orphan links, and
  purge failures;
- tests that verify benchmark mode fails closed when required source slices are
  unavailable.

## Implementation Plan

Slice 1: migration and metadata writes

- Add the metadata/link tables.
- Add insertion helpers with explicit mode input.
- Add tests for default-off behavior and policy-blocked source slices.

Slice 2: benchmark integration

- Add mode metadata to coding benchmark reports.
- Link fixture episode ids and seeded remem evidence to benchmark runs.
- Extend report validation to distinguish missing source evidence from
  retrieval miss.

Slice 3: read surfaces

- Add compact metadata to raw search, detail/current-state, and REST responses.
- Keep raw expansion explicit and policy-gated.

Slice 4: governance and observability

- Add purge and linked-slice governance behavior.
- Add doctor/status checks and eval-gate coverage.

## Validation

Spec-only PR:

```bash
python3 scripts/ci/check_spec_lifecycle.py
cargo fmt --check
cargo check
```

Implementation PR examples:

```bash
cargo test high_fidelity --lib
cargo test coding_bench --lib
cargo run -- eval-coding-bench --fixture eval/coding-bench/fixtures/tasks.json --runs-per-condition 1 --dry-run
cargo run -- eval-gates --json-out /tmp/remem-eval-gates.json
```
