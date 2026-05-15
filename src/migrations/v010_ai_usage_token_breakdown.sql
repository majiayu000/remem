ALTER TABLE ai_usage_events ADD COLUMN reasoning_tokens INTEGER NOT NULL DEFAULT 0;
ALTER TABLE ai_usage_events ADD COLUMN cache_creation_tokens INTEGER NOT NULL DEFAULT 0;
ALTER TABLE ai_usage_events ADD COLUMN cache_read_tokens INTEGER NOT NULL DEFAULT 0;
ALTER TABLE ai_usage_events ADD COLUMN raw_input_tokens INTEGER NOT NULL DEFAULT 0;
ALTER TABLE ai_usage_events ADD COLUMN raw_output_tokens INTEGER NOT NULL DEFAULT 0;
ALTER TABLE ai_usage_events ADD COLUMN usage_source TEXT NOT NULL DEFAULT 'text_estimate';
ALTER TABLE ai_usage_events ADD COLUMN pricing_source TEXT NOT NULL DEFAULT 'remem_static';

UPDATE ai_usage_events
SET raw_input_tokens = input_tokens,
    raw_output_tokens = output_tokens
WHERE raw_input_tokens = 0
  AND raw_output_tokens = 0
  AND (input_tokens > 0 OR output_tokens > 0);
