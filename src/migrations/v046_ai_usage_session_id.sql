-- v046_ai_usage_session_id: Attribute future AI usage rows to the session
-- that triggered the memory work so status can report latest-session spend.

ALTER TABLE ai_usage_events ADD COLUMN session_id TEXT;

CREATE INDEX IF NOT EXISTS idx_ai_usage_session_created
    ON ai_usage_events(session_id, created_at_epoch DESC);
