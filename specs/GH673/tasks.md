# Task Plan

## Linked Issue

GH-673

## Spec Packet

- Product: `product.md`
- Tech: `tech.md`
- Authoritative docs contract:
  `docs/specs/cache-stable-injection/PRODUCT.md` and
  `docs/specs/cache-stable-injection/TECH.md`

## Implementation Tasks

- [ ] `SP673-T1` Owner: agent; Dependencies: spec approval; Done when: the context renderer exposes a `render_contract_version` constant, includes it in eval output, and feeds it into injection-gate data-version hashing; Verify: gate-hash and eval JSON shape tests.
- [ ] `SP673-T2` Owner: agent; Dependencies: `SP673-T1`; Done when: `src/context/render.rs` no longer reads wall-clock state or emits relative timestamps, run-local counters, or volatile footer data inside the stable prefix; Verify: forbidden-pattern renderer unit test.
- [ ] `SP673-T3` Owner: agent; Dependencies: `SP673-T2`; Done when: render input construction resolves any age-sensitive labels before rendering and all renderer item lists use total stable ordering with explicit tie-break keys; Verify: equal-score and equal-priority ordering tests.
- [ ] `SP673-T4` Owner: agent; Dependencies: `SP673-T3`; Done when: context budget enforcement drops complete items from stable tail positions and never truncates at incidental byte offsets; Verify: deterministic truncation regression tests.
- [ ] `SP673-T5` Owner: agent; Dependencies: `SP673-T2` `SP673-T3` `SP673-T4`; Done when: an unchanged fixture database renders byte-identical SessionStart blocks across consecutive runs; Verify: CI fixture determinism test with zero-byte diff.
- [ ] `SP673-T6` Owner: agent; Dependencies: `SP673-T5`; Done when: the one-memory-added churn eval reports changed-byte count and asserts unchanged-prefix preservation before the first affected section; Verify: churn eval test and JSON snapshot.
- [ ] `SP673-T7` Owner: agent; Dependencies: `SP673-T5`; Done when: prompt-time/additional-context injection renders as an additive block after the SessionStart prefix without mutating prefix bytes; Verify: additive layering integration test.
- [ ] `SP673-T8` Owner: agent; Dependencies: `SP673-T1` `SP673-T2` `SP673-T3` `SP673-T4` `SP673-T5` `SP673-T6` `SP673-T7`; Done when: docs or release notes mention normalized context layout and local verification passes; Verify: `cargo fmt --check`, `cargo check`, focused context/eval tests, and `cargo test` before merge readiness.

## Parallelization

Implementation should start serially through `SP673-T4` because renderer
purity, input labels, ordering, and truncation touch the same context boundary.
After that boundary is stable, churn eval work (`SP673-T6`) and prompt-time
layering tests (`SP673-T7`) may proceed in parallel if their writable files are
kept disjoint.

## Verification

- `git diff --check`
- `python3 checks/check_workflow.py --repo .`
- `python3 checks/check_workflow.py --repo . --spec-dir specs/GH673`
- `cargo fmt --check`
- `cargo check`
- Focused context renderer, injection-gate, and eval churn tests
- `cargo test` before merge readiness

## Handoff Notes

Use `Refs #673` for spec-only or partial implementation PRs. Do not close
GH-673 until every acceptance criterion in `product.md` is implemented and
verified. The current issue state is `ready_to_spec`; implementation still
requires human spec approval and transition to `ready_to_implement`.
