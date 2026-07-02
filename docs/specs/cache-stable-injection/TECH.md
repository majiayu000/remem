# Cache-Stable Injection Technical Spec

Status: Current contract
Date: 2026-07-02

Tracking:
- Spec/tracking issue: #673

## Existing Implementation Facts

- Rendering: `src/context/render.rs` (`render_context_output_from_inputs`)
  assembles header, retrieval hints, citation contract line, Preferences,
  Lessons, Core, Memory Index, Workstreams, Sessions, and an owner-count
  footer; inputs come from `src/context/render_inputs.rs` and hybrid
  retrieval (`src/context/hybrid_context.rs`).
- The header currently includes a human-readable "updated" time; staleness
  labels derive from age (`src/memory/staleness.rs`).
- The injection gate (`src/context/injection_gate/`) hashes a data version and
  suppresses identical re-injection across sessions (v039
  `context_injection_items`); it deduplicates, it does not normalize bytes.
- Budgets and section caps live in `src/context/policy.rs`.
- Eval surfaces exist under `src/eval/` with JSON output and CI gates
  (`eval-gates`).

## Design Rules

- Rendered output is a pure function of (memory selection, render-contract
  version); anything else is a defect.
- Wall-clock reads are confined to selection (decay, staleness); the renderer
  receives already-resolved labels and never calls time functions itself.
- Ordering keys are total: (score bucket, memory id), never HashMap iteration
  order.
- Truncation cuts at item boundaries only.

## Proposed Design

### Renderer determinism pass

1. Inventory volatile fields in `render.rs` output: relative/humanized times,
   run-local counts, any formatting derived from `now`. Replace with either
   absolute epoch-derived stable labels (date, not time-of-day, where a label
   is needed) or move them out of the stable prefix (footer after all stable
   sections, or removed).
2. Thread a `RenderClock` (resolved timestamps/labels computed in
   `render_inputs.rs`) into the renderer so `render.rs` itself is
   time-free and can be property-tested for purity.
3. Normalize ordering: all section item sorts get an explicit secondary key
   (memory id ascending). Score-based ordering uses bucketed scores
   (quantized to a documented precision) so sub-noise score drift cannot
   reorder items.
4. Deterministic truncation: `policy.rs` budget enforcement drops whole items
   from the tail of each section in stable order.
5. Add `render_contract_version: u32` constant; include it in eval JSON and in
   the injection gate's hashed data version so a contract bump naturally
   re-injects.

### Session layering

- SessionStart output is the stable prefix.
- UserPromptSubmit retrieval injection (additionalContext) renders as a
  separate additive block with its own deterministic contract, appended after
  the prefix; it never causes a re-render of the prefix.

### Churn eval

New eval check (wired into `eval-gates`):

- `block_churn_unchanged`: render fixture DB twice, byte-diff must be 0.
- `block_churn_one_added`: render, insert one fixture memory, render again;
  report changed-byte count and assert the prefix up to the first affected
  section is byte-identical.
- Output includes `render_contract_version`.

## Product-to-Test Mapping

| Product invariant | Implementation area | Verification |
| --- | --- | --- |
| P1 byte identity | render.rs purity | eval `block_churn_unchanged` = 0 in CI |
| P2 no volatile fields | RenderClock refactor | unit test scanning fixture output for forbidden patterns |
| P3 total ordering | section sorts | unit test: equal-score items order by id |
| P4 localized change | section assembly | eval `block_churn_one_added` prefix assertion |
| P5 additive prompt block | UserPromptSubmit path | integration test: prefix bytes unchanged after prompt injection |
| P6 versioned contract | render_contract_version | eval JSON contains version; gate hash includes it |

## Data Flow

DB state -> selection (time-aware, in render_inputs) -> resolved RenderInputs
(time-free) -> deterministic renderer -> stable prefix; prompt-time retrieval
-> additive block. Eval consumes renderer directly against fixture DBs.

## Alternatives Considered

- Pre-building the block in the background worker and serving it verbatim at
  SessionStart: complementary (and cache-friendly), but not required for the
  determinism contract; deferred to consolidation work under #383 to keep this
  change renderer-scoped.
- Hash-only stability (keep bytes volatile, rely on gate dedup): rejected;
  the host's prompt cache sees bytes, not remem's hashes.

## Risks

- Security: none new; output content unchanged in substance.
- Compatibility: downstream parsers of the current block layout (tests,
  SessionStart snapshot assertions) need updating in the same PR; the
  render-contract version makes this auditable.
- Performance: neutral or better (renderer does less time formatting).
- Maintenance: new renderer code must respect purity; the forbidden-pattern
  unit test guards regressions.

## Test Plan

- [ ] Unit tests: renderer purity (no time calls — enforced by construction
      via RenderClock), ordering keys, deterministic truncation, forbidden
      volatile patterns.
- [ ] Integration tests: churn evals on fixture DBs; UserPromptSubmit
      layering.
- [ ] Manual verification: two real `remem context` runs on an idle project,
      `diff` shows zero bytes; run after one real session, diff is localized.

## Rollback Plan

Revert the renderer commit; no schema or artifact changes. The
render-contract version makes the revert visible in eval output.
