# Task Plan

## Linked Issue

GH-675

## Implementation Issue

GH-692

## Spec Packet

- Product: `product.md`
- Tech: `tech.md`
- Authoritative docs contract:
  `docs/specs/capacity-eval-axis/PRODUCT.md` and
  `docs/specs/capacity-eval-axis/TECH.md`

## Tasks

- [ ] `SP675-T1` Owner: agent; Dependencies: none; Done when: `specs/GH675` validates and GH-692 is linked as the implementation issue; Verify: `python3 checks/check_workflow.py --repo . --spec-dir specs/GH675`.
- [ ] `SP675-T2` Owner: agent; Dependencies: `SP675-T1`; Done when: deterministic scaled fixture synthesis appends unique non-relevant noise and computes stable corpus hashes; Verify: capacity determinism tests.
- [ ] `SP675-T3` Owner: agent; Dependencies: `SP675-T2`; Done when: capacity report includes per-scale fused metrics, p95 latency, and largest-scale degradation; Verify: JSON shape and degradation tests.
- [ ] `SP675-T4` Owner: agent; Dependencies: `SP675-T3`; Done when: `remem eval-capacity` exposes dataset, seed, scales, k, json-out, and json flags; Verify: CLI parsing and command smoke.
- [ ] `SP675-T5` Owner: agent; Dependencies: `SP675-T2` `SP675-T3` `SP675-T4`; Done when: local deterministic checks and focused Rust tests pass; Verify: commands below.

## Parallel Split

No parallel writable lanes for this first slice. The generator, report, and CLI
touch the same eval surface and should land as one implementation PR.

## Verification

- `git diff --check`
- `python3 checks/check_workflow.py --repo .`
- `python3 checks/check_workflow.py --repo . --spec-dir specs/GH675`
- `cargo fmt --check`
- `cargo check --message-format=short`
- `cargo test capacity`
- `cargo test cli_parses_eval_capacity_options`
- `cargo test`

## Handoff Notes

Use `Refs #675` and `Closes #692` in the implementation PR. Do not close
GH-675 until per-channel attribution, gating, dashboard ingestion, and measured
degradation documentation are complete.
