# Task Plan

## Linked Issue

GH-882

## Spec Packet

- Product: `product.md`
- Tech: `tech.md`

## Implementation Tasks

- [x] `SP882-T1` Owner: coordinator; Dependencies: none; Done when: `fact` becomes `discovery`, valid neighboring candidates survive, all seven canonical types round-trip, and unrelated unknown values still error; Verify: `cargo test memory_candidate::parse::tests --lib`.
- [x] `SP882-T2` Owner: coordinator; Dependencies: `SP882-T1`; Done when: the system and task prompt dynamically list all seven canonical values and direct factual findings to `discovery` instead of `fact`; Verify: `cargo test memory_candidate_prompt_names_canonical_types_and_maps_fact --lib`.
- [x] `SP882-T3` Owner: coordinator; Dependencies: `SP882-T1`, `SP882-T2`; Done when: the diff contains no unrelated behavior or issue scope and no extra user-facing documentation is needed beyond this packet; Verify: `git diff --check` and scoped diff inspection.

## Parallelization

Implementation is serial in one worktree because both production changes share
the memory-candidate module and its prompt/parser contract. After the
coordinator has a stable diff, one native reviewer lane owns read-only
inspection of GH-882; it must not edit files or run Cargo. The coordinator is
the sole full verification owner.

## Verification

- `cargo test memory_candidate::parse::tests --lib`
- `cargo test memory_candidate_prompt_names_canonical_types_and_maps_fact --lib`
- `python3 checks/check_workflow.py --repo .`
- `python3 checks/check_workflow.py --repo . --spec-dir specs/GH882`
- `cargo fmt --check`
- `cargo check`
- `cargo test`
- `cargo clippy --all-targets -- -D warnings`
- `python3 scripts/ci/check_pr_preflight.py --base origin/main --pr-body-file /tmp/pr-body-gh882.md`
- Current-head PR checks and SpecRail PR gate pass with no unresolved review threads.

## Handoff Notes

- Queue mode is `bounded_tranche`; this packet and implementation cover only
  GH-882.
- `auth_mode: auto` is standing merge authorization for this run, but does not
  weaken CI, independent review, review-thread, or PR-gate requirements.
- Keep `MemoryType::from_observation_type` strict; `fact` is a parser alias, not
  a legal observation type.
- Do not retry production quarantine ranges as part of this change.
