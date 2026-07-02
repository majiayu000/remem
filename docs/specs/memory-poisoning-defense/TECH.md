# Memory Poisoning Defense Technical Spec

Status: Current contract
Date: 2026-07-02

Tracking:
- Spec/tracking issue: #672

## Existing Implementation Facts

- Capture provenance is already recorded: `captured_events` keeps host,
  session, and event identity (`src/migrations/v006_capture_pipeline.sql`);
  candidates keep evidence ids; `memory_facts` keeps `source_event_ids`.
- Secret redaction runs in the capture adapters
  (`src/adapter/redaction.rs`); it does not classify instructions.
- The auto-promote gate (`src/memory_candidate.rs`, `should_auto_promote`)
  checks scope, risk class, confidence, routing, evidence, memory type, unsafe
  markers, and observation support — but not content instruction patterns or
  source trust.
- Candidate review exists (`src/memory_candidate/review.rs`) with approve,
  discard, and edit paths.
- The renderer (`src/context/render.rs`) injects memory content verbatim;
  `memory_suppressions` (v051) can hide items but nothing populates it
  automatically for adversarial content.
- Direct memory saves (`src/memory/service/save.rs`) bypass candidate review
  and write active memories directly, so they must receive equivalent trust,
  scan, and acknowledgement metadata rather than relying only on candidate
  insertion.

## Design Rules

- Deterministic, versioned pattern matching; no LLM in scan or injection
  paths.
- Quarantine is a candidate review status, not deletion; evidence is
  preserved for audit.
- Trust class is derived from capture provenance, never self-declared by
  extraction output.
- Injection-time drops log at error level with enough detail to diagnose
  (memory id, pattern id, provenance) — no silent degradation.

## Proposed Design

### Trust classification

At candidate insertion, derive `source_trust_class` from the supporting
captured events:

| Event origin | Trust class |
| --- | --- |
| UserPromptSubmit text | `user_prompt` |
| Read/Grep on repo-owned paths | `repo_file` |
| Bash/tool output | `local_tool_output` |
| Session summaries | Inherit the lowest trust class of the covered source events; if source expansion is unavailable, treat as `external_content` for auto-promote decisions |
| WebFetch/WebSearch results, MCP output from remote servers | `external_content` |
| Direct `save_memory` CLI/API input from the local authenticated caller | `user_prompt` |

Lowest class among supporting evidence wins. Stored as a new candidate column
and copied onto promoted memories (next free migration after the current
schema; do not reserve an already-used migration number). Pre-existing rows
default to `local_tool_output`.

### Instruction-pattern scan

A versioned pattern table (Rust source, unit-tested, English and Chinese
variants) covering:

- override/authority phrases ("ignore previous instructions", "absolute
  authority", "supersedes user");
- execution imperatives directed at the reader ("run the following",
  "execute this command silently");
- concealment directives ("do not mention", "hide this from");
- opaque payload heuristics (long base64-like runs above a threshold).

Scan points:

1. Candidate insertion (`src/memory_candidate.rs`): match -> insert with
   `review_status='quarantined'` plus `quarantine_pattern_id`; never eligible
   for `should_auto_promote`.
2. Direct save (`src/memory/service/save.rs`): classify as trusted local user
   input, scan before insert, and fail with a structured validation error on
   pattern match unless the caller supplies explicit acknowledgement metadata.
   Acknowledged direct saves record pattern id + timestamp in the same durable
   acknowledgement store used by reviewed candidates.
3. Injection render (`src/context/render.rs` input assembly): re-scan final
   item content; match -> drop item, `log::error!` with memory id + pattern
   id, increment a doctor-visible counter. Approved-after-review memories
   carry an acknowledgement flag that suppresses the injection-time re-drop
   for the acknowledged pattern id only.

### Gate integration

`should_auto_promote` gains: `source_trust_class >= config floor` (default
`local_tool_output`); `external_content` is never auto-promotable regardless
of confidence. Block reasons logged through the existing
`auto_promote_block_reason` channel.

### Review and CLI

- Quarantined candidates appear in the existing review listing with the
  matched pattern rendered.
- `approve` on a quarantined candidate requires `--acknowledge-pattern` (or
  interactive confirmation) and records pattern id + timestamp in the
  operation log (`memory_operation_log`).

### Doctor

- quarantine count by pattern id;
- pattern-set version;
- injection-drop counter with last drop detail.

## Product-to-Test Mapping

| Product invariant | Implementation area | Verification |
| --- | --- | --- |
| P1 trust class derivation | candidate insertion | unit tests per origin; mixed-evidence lowest-wins test |
| P2 quarantine on match | memory_candidate | fixture: override phrase -> quarantined, gate never sees it |
| P3 trust floor in gate | should_auto_promote | boundary tests incl. external_content never promotes |
| P4 injection re-scan | context render | fixture: flagged row in DB -> absent from output, error logged |
| P5 acknowledged approval | review path | test: approve records pattern ack; item renders afterward |
| P6 determinism | pattern module | table-driven tests; version bump changes verdicts only via table |
| P7 direct save coverage | save_memory service | test: direct save gets trust class; pattern match requires acknowledgement |

## Data Flow

captured_events (with origin) -> candidate insertion (trust class + scan) ->
quarantine or pending/auto-promote -> review (acknowledged approval) ->
memories (trust class copied) -> render-time re-scan -> context block. Drops
flow to log + doctor counters.

Direct save path: local caller input -> trust class `user_prompt` + pattern
scan -> either structured validation error or acknowledged active memory ->
render-time re-scan with acknowledgement check.

## Alternatives Considered

- LLM-based intent classification at write time: deferred; adds cost and
  non-determinism; the pattern layer is the enforceable baseline (mirrors the
  SEC-14-style first-pass model), and a semantic layer can compose later.
- Deleting matched candidates outright: rejected; destroys audit evidence and
  removes the false-positive escape hatch.
- Scanning only at injection time: rejected; a poisoned memory would still
  reach the store, MCP `search`, and export surfaces.

## Risks

- Security: pattern lists are bypassable by paraphrase; trust floor is the
  backstop (external content cannot auto-promote at all). Documented as
  defense-in-depth, not a guarantee.
- Compatibility: the next free migration adds columns with defaults; old rows
  readable unchanged.
- Performance: regex table scan per candidate and per rendered item; bounded
  and measured by existing latency benchmarks.
- Maintenance: pattern table growth is versioned and unit-tested; MCP tool
  output classification depends on adapter origin fidelity.

## Test Plan

- [ ] Unit tests: pattern table (positive/negative per pattern), trust
      derivation, gate floor boundaries.
- [ ] Integration: seeded poisoned captured_events fixture end-to-end
      (capture -> extraction -> quarantine -> render absence -> doctor).
- [ ] Manual verification: real session with a fetched web page containing an
      override phrase; confirm quarantine and doctor visibility.

## Rollback Plan

Config flags disable the injection re-scan and the trust floor independently;
quarantined rows can be bulk-moved back to `pending_review` with a one-line
SQL update documented in the migration notes. The additive migration columns
can remain in place when the feature is disabled.
