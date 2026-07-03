# Tech Spec

## Linked Issue

GH-673

## Product Spec

Link to `product.md`.

## Accepted Contract

The authoritative technical contract is
`docs/specs/cache-stable-injection/TECH.md`.

This SpecRail packet reflects the existing #673 contract and keeps
implementation behind the normal SpecRail readiness and spec-approval gates.

## Codebase Context

| Area | Files | Current behavior | Why relevant |
| --- | --- | --- | --- |
| Context renderer | `src/context/render.rs` | Assembles the injected block, including header, preferences, lessons, core memories, memory index, workstreams, sessions, and footer. | This is the byte surface that must become deterministic for unchanged inputs. |
| Render inputs | `src/context/render_inputs.rs`, `src/context/hybrid_context.rs` | Loads and prepares selected memories, session summaries, workstreams, and resolved metadata for rendering. | Time-aware selection should happen before rendering so the renderer stays pure. |
| Context policy | `src/context/policy.rs` | Applies budgets and section caps for context output. | Truncation must be deterministic and item-boundary based. |
| Injection gate | `src/context/injection_gate/` | Hashes data versions and suppresses identical re-injection. | The render-contract version must participate in the gate hash so intentional format changes re-inject. |
| Staleness labels | `src/memory/staleness.rs` | Derives age-sensitive labels. | Any age-derived value must be resolved before rendering and must not introduce per-run byte churn. |
| Eval gates | `src/eval/`, `eval/`, `src/cli/actions/eval.rs` | Existing eval commands emit JSON and feed CI gates. | Add block-churn metrics and render-contract version to deterministic eval output. |
| Tests | `tests/`, `src/context/**/tests.rs`, `src/eval/**/tests.rs` | Snapshot and unit tests cover current rendering and eval behavior. | New tests must prove byte identity, forbidden volatile fields, deterministic ordering, additive layering, and JSON shape. |

## Design Rules

- Rendering is a pure function of selected memory state plus
  `render_contract_version`.
- `render.rs` must not call wall-clock APIs or depend on unordered collection
  iteration.
- Ordering keys are total and stable, with explicit tie-breaks such as memory
  id or topic key. Score-based ordering must use documented buckets or
  quantization before tie-break keys.
- Budget enforcement drops complete items from the tail of a stable order.
- Prompt-time injection renders as a separate additive block after the
  SessionStart prefix, and that block has its own deterministic contract for
  identical prompt and retrieval inputs.

## Proposed Design

1. Add a renderer contract constant, for example
   `RENDER_CONTRACT_VERSION: u32`, near the context rendering boundary.
2. Audit `src/context/render.rs` for volatile fields. Remove relative
   timestamps and run-local counters from the stable prefix or replace them
   with stable state-derived labels.
3. Move time-sensitive label resolution into render input construction so the
   renderer receives resolved labels and does not read the current time.
4. Normalize section ordering by adding documented score buckets or
   quantization before explicit secondary keys for every list that can have
   equal score, priority, or timestamp.
5. Update budget/truncation behavior so items are kept or dropped whole after
   stable ordering is established.
6. Include `render_contract_version` in the injection gate data-version hash
   and in eval JSON output.
7. Add a churn eval path wired into `eval-gates` that renders fixture state
   twice for the unchanged case, then renders after adding one fixture memory
   and reports the changed bytes plus unchanged-prefix assertion.
8. Add or update tests for forbidden volatile tokens, score bucketing,
   deterministic ordering, item-boundary truncation, additive and deterministic
   prompt-time injection, and eval JSON shape.

## Product-to-Test Mapping

| Product invariant | Implementation area | Verification |
| --- | --- | --- |
| P1 byte identity | `src/context/render.rs`, fixture render harness | Determinism test renders unchanged fixture inputs twice and compares bytes. |
| P2 no volatile fields | render input boundary, renderer output tests | Unit test scans stable prefix for relative time/counter patterns. |
| P3 deterministic order | section sort keys, score buckets, and stable tie-breaks | Equal-score and near-score fixture tests assert bucketed ordering before memory-id or topic-key tie-breaks. |
| P4 localized delta | churn eval fixture | One-memory-added eval asserts unchanged prefix before the first affected section and permits every logically affected section to change. |
| P5 additive prompt block | prompt-time injection renderer/path | Integration test asserts SessionStart prefix bytes are unchanged after additive injection and repeated prompt-time renders are byte-identical for identical prompt/retrieval inputs. |
| P6 versioned contract | render constant, injection gate, eval JSON | JSON shape test and gate hash test include `render_contract_version`. |

## Data Flow

Database state and host inputs feed context selection. Selection and input
construction resolve age-sensitive labels and stable ordering metadata. The
renderer consumes those resolved inputs plus `render_contract_version` and emits
the SessionStart stable prefix. Prompt-time retrieval renders a separate block
that is appended after the prefix and remains deterministic for identical
prompt-time inputs. Eval commands consume the same renderer boundary and emit
block-churn metrics through `eval-gates`.

## Alternatives Considered

- Prebuild and store the entire context block in the background worker:
  deferred because byte determinism can be proven at the renderer boundary
  without adding persistence or cache invalidation state.
- Rely only on injection gate hashes: rejected because providers cache prompt
  bytes, not remem's internal hash decisions.
- Keep relative timestamps after quantizing them: rejected for the stable
  prefix because unchanged memory state should produce zero byte churn.

## Risks

- Security: No new secrets, network calls, or authorization surfaces.
- Compatibility: Existing renderer snapshots and downstream parsers may need
  updates because layout normalization can change cosmetic output.
- Performance: Neutral or slightly better; deterministic sorting and whole-item
  truncation should stay within existing context-render budgets.
- Maintenance: Future renderer edits must preserve purity and stable ordering;
  tests should make volatile fields and unordered ties fail loudly.

## Test Plan

- [ ] Unit tests: renderer purity boundary, forbidden volatile fields,
      score bucket ordering, equal-score ordering, and deterministic
      item-boundary truncation.
- [ ] Integration tests: unchanged fixture byte identity, one-memory-added
      localized churn, and prompt-time additive deterministic layering.
- [ ] Eval tests: block-churn JSON contains `render_contract_version`,
      unchanged bytes are zero, one-memory-added prefix preservation is
      asserted, and the check is wired into `eval-gates`.
- [ ] Manual verification: run `remem context` twice on an idle project and
      compare bytes; then add one memory and confirm the diff is localized.
- [ ] Repository verification: `cargo fmt --check`, `cargo check`, focused
      context/eval tests, and `cargo test` before merge readiness.

## Rollback Plan

Revert the renderer, gate-hash, and eval changes from the implementation PR.
No schema migration or persisted data rewrite is part of this contract.
