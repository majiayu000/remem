-- v092_session_intent_display: optional session/workstream intent + topic.
--
-- MMDD display dates are derived at read time from created epochs. Unknown
-- stored intent/source values are rejected by CHECK constraints. Model output
-- abstains in application code before it reaches these writers.

ALTER TABLE session_summaries ADD COLUMN session_intent TEXT
    CHECK(
        session_intent IS NULL OR session_intent IN (
            'fea', 'des', 'fix', 'opt', 'rel', 'exp', 'doc', 'res'
        )
    );
ALTER TABLE session_summaries ADD COLUMN session_topic TEXT;
ALTER TABLE session_summaries ADD COLUMN session_intent_source TEXT
    CHECK(
        session_intent_source IS NULL OR session_intent_source IN (
            'summary', 'override', 'rollup'
        )
    );
ALTER TABLE session_summaries ADD COLUMN session_intent_updated_at_epoch INTEGER;

ALTER TABLE workstreams ADD COLUMN session_intent TEXT
    CHECK(
        session_intent IS NULL OR session_intent IN (
            'fea', 'des', 'fix', 'opt', 'rel', 'exp', 'doc', 'res'
        )
    );
ALTER TABLE workstreams ADD COLUMN session_topic TEXT;
ALTER TABLE workstreams ADD COLUMN session_intent_source TEXT
    CHECK(
        session_intent_source IS NULL OR session_intent_source IN (
            'summary', 'override', 'rollup'
        )
    );
ALTER TABLE workstreams ADD COLUMN session_intent_updated_at_epoch INTEGER;
