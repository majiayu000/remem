# Tech Spec

## Linked Issue

GH-672

## Product Spec

Link to `product.md`.

## Accepted Contract

The authoritative technical contract is
`docs/specs/memory-poisoning-defense/TECH.md`.

This SpecRail packet reflects the existing #672 contract and keeps
implementation behind the normal SpecRail readiness and spec-approval gates.

## Codebase Context

| Area | Files | Current behavior | Why relevant |
| --- | --- | --- | --- |
| Capture provenance | `src/db/capture.rs`, `src/db/extraction.rs` | Captured events and extraction tasks carry host/session/evidence identity. | Trust class derives from provenance, not model claims. |
| Candidate gate | `src/memory_candidate.rs`, `src/memory_candidate/review.rs` | Auto-promote checks confidence, risk, routing, evidence, and support. | Add trust floor, quarantine status, and acknowledgement review path. |
| Direct save | `src/memory/service/save.rs` | Authenticated local saves can write active memories directly. | Direct writes need equivalent scan and acknowledgement handling. |
| Renderer | `src/context/render.rs`, `src/context/render_inputs.rs` | Active memories are rendered into context. | Injection-time re-scan drops unacknowledged poisoned content. |
| Doctor | `src/doctor/` | Reports runtime health and actionable queues. | Must expose quarantine and injection-drop state. |
| Migrations | `src/migrations/`, `src/migrate/` | Schema evolves through versioned additive migrations. | Trust and quarantine metadata require migration tests. |

## Design Rules

- Pattern matching is deterministic, versioned, and local.
- Quarantine preserves evidence and stays reviewable.
- Source trust is derived from capture provenance and cannot be self-declared
  by extraction output.
- Injection-time drops log at error level with memory id and pattern id.
- Direct saves cannot bypass the scan.

## Proposed Design

1. Add a source trust enum for candidate and memory rows:
   `user_prompt`, `repo_file`, `local_tool_output`, `external_content`, with
   lowest-supported evidence winning.
2. Add a versioned instruction-pattern module covering override phrases,
   execution imperatives, concealment directives, authority claims, and opaque
   payload heuristics.
3. On candidate insertion, store `quarantine_pattern_id` and route matches to
   a quarantined review state; never pass them through auto-promote.
4. Extend auto-promote with a configurable trust floor and explicit block
   reasons.
5. Extend direct save to scan before insertion and require durable
   acknowledgement metadata for matched content.
6. Re-scan final render inputs and drop unacknowledged matches with loud
   logging plus doctor-visible counters.
7. Extend review approval for quarantined candidates to require explicit
   pattern acknowledgement and persist that acknowledgement.

## Product-to-Test Mapping

| Product invariant | Implementation area | Verification |
| --- | --- | --- |
| Trust derived from provenance | candidate insertion | unit tests per origin and mixed-evidence lowest-wins |
| Matches quarantine | memory candidate write path | fixture: override phrase becomes quarantined |
| Trust floor gates auto-promote | auto-promote gate | boundary tests including external content |
| Render defense drops misses | context render inputs | poisoned active row absent from output, error logged |
| False positive approval | review path | approve requires acknowledgement and later renders |
| Direct save coverage | save service | pattern match returns structured error unless acknowledged |

## Data Flow

Captured events feed extraction tasks and candidate insertion. Candidate
insertion derives trust and scans content. Clean candidates continue to review
or auto-promote; matches become quarantined review items. Approved candidates
copy trust and acknowledgement metadata into memories. Render input assembly
re-scans final content and drops unacknowledged matches before context output.

## Risks

- Security: pattern matching is bypassable; trust floor prevents low-trust
  auto-promotion and render-time scan is defense in depth.
- Compatibility: additive schema changes must keep old rows readable.
- Performance: scans are bounded and must be covered by focused latency tests
  if the pattern table grows.

## Test Plan

- [ ] Unit tests for pattern positives/negatives and versioning.
- [ ] Trust-derivation and auto-promote floor tests.
- [ ] Candidate quarantine and review acknowledgement tests.
- [ ] Direct-save scan and acknowledgement tests.
- [ ] Context render drop and doctor counter tests.
- [ ] `cargo fmt --check`, `cargo check`, focused tests, and `cargo test`
      before merge readiness.

## Rollback Plan

Feature flags may disable the trust floor and render re-scan. Additive columns
remain in place. Quarantined rows can be moved back to pending review only by
an explicit maintenance command or documented SQL procedure.
