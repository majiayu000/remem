# Task Plan

## Linked Issue

GH-672

## Spec Packet

- Product: `product.md`
- Tech: `tech.md`
- Authoritative docs contract:
  `docs/specs/memory-poisoning-defense/PRODUCT.md` and
  `docs/specs/memory-poisoning-defense/TECH.md`

## Implementation Tasks

- [ ] `SP672-T1` Owner: agent; Dependencies: spec approval; Done when: the trust-class vocabulary is modeled and persisted for candidates and promoted memories with additive migrations; Verify: migration and compatibility tests.
- [ ] `SP672-T2` Owner: agent; Dependencies: `SP672-T1`; Done when: deterministic instruction-pattern scanning exists with pattern-set version and table-driven positive/negative tests; Verify: pattern module tests.
- [ ] `SP672-T3` Owner: agent; Dependencies: `SP672-T1` `SP672-T2`; Done when: candidate insertion derives trust from evidence and quarantines pattern matches with durable pattern id; Verify: candidate write tests.
- [ ] `SP672-T4` Owner: agent; Dependencies: `SP672-T3`; Done when: auto-promote consumes the trust floor and records clear block reasons; Verify: auto-promote boundary tests.
- [ ] `SP672-T5` Owner: agent; Dependencies: `SP672-T2`; Done when: direct `save_memory` scans content and requires explicit acknowledgement for matched patterns; Verify: save service tests.
- [ ] `SP672-T6` Owner: agent; Dependencies: `SP672-T2` `SP672-T3`; Done when: render input assembly drops unacknowledged poisoned content and logs error-level diagnostic detail; Verify: context render fixture and log/doctor tests.
- [ ] `SP672-T7` Owner: agent; Dependencies: `SP672-T3`; Done when: review approval of quarantined candidates requires and records pattern acknowledgement; Verify: review path tests.
- [ ] `SP672-T8` Owner: agent; Dependencies: `SP672-T1` `SP672-T6`; Done when: doctor reports quarantine counts, pattern-set version, and last injection drop; Verify: doctor tests.
- [ ] `SP672-T9` Owner: agent; Dependencies: all implementation tasks; Done when: docs and release notes describe the security behavior and verification passes; Verify: `cargo fmt --check`, `cargo check`, focused tests, and `cargo test`.

## Parallelization

Start serially through `SP672-T3` because schema, trust derivation, and
quarantine status define the contract boundary. After that, direct-save,
render-defense, review, and doctor work can split if writable files are kept
disjoint.

## Verification

- `git diff --check`
- `python3 checks/check_workflow.py --repo .`
- `python3 checks/check_workflow.py --repo . --spec-dir specs/GH672`
- `cargo fmt --check`
- `cargo check`
- Focused candidate, direct-save, render, review, doctor, and migration tests
- `cargo test` before merge readiness

## Handoff Notes

Use `Refs #672` for spec-only or partial implementation PRs. Do not close
GH-672 until every acceptance criterion in `product.md` is implemented and
verified. This packet requires human spec approval and transition to
`ready_to_implement` before runtime implementation.
