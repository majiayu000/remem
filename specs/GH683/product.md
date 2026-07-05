# Product Spec

## Linked Issue

GH-683

## Accepted Contract

The authoritative product contract is
`docs/specs/review-queue-throughput/PRODUCT.md`.

This SpecRail packet hands the accepted #683 contract to implementation. It
does not replace the `docs/specs/` contract.

## User Problem

The memory candidate promotion gate is intentionally strict, so many useful
candidates remain in `pending_review`. Without queue health metrics, batch
review tools, and deadlock reporting, the curation queue can silently age and
memory growth stalls while capture continues.

## Goals

- Expose review queue health in `remem status --json` and doctor output.
- Add filter-scoped batch review operations with preview/confirm behavior.
- Surface aggregate block reasons so systematic promotion deadlocks are visible.
- Preserve durable review provenance for single and batch outcomes.
- Keep REST candidate surfaces aligned with CLI filters and block-reason data.

## Non-Goals

- Do not loosen or retune auto-promotion gates.
- Do not auto-approve candidates.
- Do not build a web review UI in this slice.
- Do not change candidate extraction behavior.

## Behavior Invariants

1. Queue metrics report pending count, median/max age, 7-day inflow/resolved
   counts, per-project splits, and block-reason aggregates.
2. Doctor warns when queue age, backlog growth, or structural block-reason
   dominance crosses documented thresholds.
3. Batch approve/discard resolves the same candidate set for preview and
   mutation, requires confirmation unless `--yes` is supplied, and uses a
   bounded default limit.
4. Batch mutations are transactionally all-or-nothing and record review actor,
   review time, action source, batch id, and optional reason.
5. REST candidate filters and block-reason reporting expose the same review
   queue slices needed by a future UI.

## Acceptance Criteria

- [ ] `status --json` exposes a `review_queue` object with total and
      per-project health metrics plus block-reason aggregates.
- [ ] `doctor` emits review queue findings for stale queues, fast-growing
      backlogs, and dominant structural block reasons.
- [ ] `remem review approve-batch`, `discard-batch`, and `blocked` exist with
      the filters defined by the accepted contract.
- [ ] Batch preview and mutation use the same filter resolution path and batch
      mutation rollback prevents partial outcomes.
- [ ] Single and batch review outcomes persist review actor/source/batch
      metadata on the candidate row.
- [ ] REST candidate list filters include type, block reason, topic, text,
      minimum confidence, and age, and REST exposes block-reason aggregates.

## Edge Cases

- Empty queues return zero metrics and no doctor warning.
- Legacy reviewed rows without `reviewed_at_epoch` use `updated_at_epoch` for
  resolved-window metrics.
- Unknown project rows remain visible in aggregate previews instead of being
  dropped.
- Non-interactive batch commands fail clearly unless `--yes` confirms mutation.

## Rollout Notes

This slice adds nullable candidate review metadata columns and an index over
`(review_status, created_at_epoch)`. Existing rows remain valid and legacy
reviewed rows are handled by the metrics fallback above.
