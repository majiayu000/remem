# Product Spec

## Linked Issue

GH-676

## Implementation Issue

GH-694

## Accepted Contract

The authoritative product contract is
`docs/specs/associative-multihop-fixtures/PRODUCT.md`.

This SpecRail packet narrows the accepted #676 contract to the first
implementation slice for GH-694. It does not replace the `docs/specs/`
contract.

## User Problem

The existing `multi_hop` golden fixtures are saturated by baseline retrieval,
so they cannot prove whether graph traversal adds value. The first slice needs
associative fixtures that baseline retrieval has real headroom on before the
literal graph traversal arm and ADR decision are meaningful.

## First Slice Goal

Ship loader-visible associative fixtures and a reproducible baseline report:

- add at least 15 `slice: "associative"` golden queries;
- document each query's intended `query -> entity -> target` path with
  `hop_path`;
- mechanically reject query/target lexical leakage;
- emit a committed baseline/headroom JSON report for the associative slice.

## Non-Goals

- Do not implement the literal `graph_edges` traversal arm in this slice.
- Do not re-run or re-record the graph ADR decision yet.
- Do not wire graph traversal into production retrieval.
- Do not claim per-channel attribution until the follow-up eval arm exists.
- Do not close GH-676 from the first-slice PR.

## Acceptance Criteria

- [ ] `eval/golden.json` includes at least 15 associative fixtures covering
      file path, crate, error signature, and issue-number entities.
- [ ] Each associative query has a loader-validated `hop_path` with source,
      entity type, entity, and target.
- [ ] Loader validation rejects associative fixtures whose query text shares
      target-content tokens after normalization.
- [ ] `remem eval-associative-baseline` writes the committed baseline report
      under `eval/associative-multihop/baseline.json`.
- [ ] Tests cover CLI parsing, fixture contract coverage, report generation,
      checked-in report parity, and overlap rejection.

## Follow-Up

GH-676 remains open for per-channel attribution, entity-BFS and literal
`graph_edges` arm deltas, trusted provenance fixture edge setup, and ADR
decision recording.
