-- v084_legacy_pending_bridge_state: halt the idle drain after residual rows are gone.
--
-- Fresh and already-empty stores must not keep admitting the
-- pending_observations bridge. Residual auto-actionable rows stay
-- frozen_draining. This is durable consumer halt only; guarded table drop
-- remains remem 0.7.0.

CREATE TABLE legacy_surface_state (
    surface TEXT PRIMARY KEY,
    state TEXT NOT NULL CHECK (state IN ('frozen_draining', 'exhausted')),
    residual_count INTEGER NOT NULL DEFAULT 0,
    exhausted_at_epoch INTEGER,
    updated_at_epoch INTEGER NOT NULL
);

INSERT INTO legacy_surface_state (
    surface, state, residual_count, exhausted_at_epoch, updated_at_epoch
)
SELECT
    'pending_observations',
    CASE WHEN residual.residual_count = 0 THEN 'exhausted' ELSE 'frozen_draining' END,
    residual.residual_count,
    CASE
        WHEN residual.residual_count = 0 THEN CAST(strftime('%s', 'now') AS INTEGER)
        ELSE NULL
    END,
    CAST(strftime('%s', 'now') AS INTEGER)
FROM (
    SELECT COUNT(*) AS residual_count
    FROM pending_observations
    WHERE host IN ('claude-code', 'codex-cli')
      AND (
            (status = 'pending'
             AND (next_retry_epoch IS NULL
                  OR next_retry_epoch <= CAST(strftime('%s', 'now') AS INTEGER)))
         OR (status = 'processing'
             AND (lease_expires_epoch IS NULL
                  OR lease_expires_epoch < CAST(strftime('%s', 'now') AS INTEGER)))
         OR (status = 'failed'
             AND COALESCE(failure_class, 'transient') = 'transient'
             AND (next_retry_epoch IS NULL
                  OR next_retry_epoch <= CAST(strftime('%s', 'now') AS INTEGER)))
      )
) AS residual;
