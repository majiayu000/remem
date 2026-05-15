UPDATE ai_usage_events
SET estimated_cost_usd = CASE
        WHEN lower(COALESCE(model, '')) LIKE '%opus-4-7%'
          OR lower(COALESCE(model, '')) LIKE '%opus-4.7%'
          OR lower(COALESCE(model, '')) LIKE '%opus-4-6%'
          OR lower(COALESCE(model, '')) LIKE '%opus-4.6%'
          OR lower(COALESCE(model, '')) LIKE '%opus-4-5%'
          OR lower(COALESCE(model, '')) LIKE '%opus-4.5%'
            THEN (input_tokens * 5.0
                + output_tokens * 25.0
                + reasoning_tokens * 25.0
                + cache_creation_tokens * 6.25
                + cache_read_tokens * 0.50) / 1000000.0
        WHEN lower(COALESCE(model, '')) LIKE '%opus%'
            THEN (input_tokens * 15.0
                + output_tokens * 75.0
                + reasoning_tokens * 75.0
                + cache_creation_tokens * 18.75
                + cache_read_tokens * 1.50) / 1000000.0
        WHEN lower(COALESCE(model, '')) LIKE '%sonnet%'
            THEN (input_tokens * 3.0
                + output_tokens * 15.0
                + reasoning_tokens * 15.0
                + cache_creation_tokens * 3.75
                + cache_read_tokens * 0.30) / 1000000.0
        WHEN lower(COALESCE(model, '')) LIKE '%haiku%'
            THEN (input_tokens * 1.0
                + output_tokens * 5.0
                + reasoning_tokens * 5.0
                + cache_creation_tokens * 1.25
                + cache_read_tokens * 0.10) / 1000000.0
        WHEN lower(COALESCE(model, '')) LIKE '%gpt-5.5%'
            THEN (input_tokens * 5.0
                + output_tokens * 30.0
                + reasoning_tokens * 30.0
                + cache_read_tokens * 0.50) / 1000000.0
        WHEN lower(COALESCE(model, '')) LIKE '%gpt-5.4-mini%'
            THEN (input_tokens * 0.75
                + output_tokens * 4.50
                + reasoning_tokens * 4.50
                + cache_read_tokens * 0.075) / 1000000.0
        WHEN lower(COALESCE(model, '')) LIKE '%gpt-5.4-nano%'
            THEN (input_tokens * 0.20
                + output_tokens * 1.25
                + reasoning_tokens * 1.25
                + cache_read_tokens * 0.020) / 1000000.0
        WHEN lower(COALESCE(model, '')) LIKE '%gpt-5.4%'
            THEN (input_tokens * 2.50
                + output_tokens * 15.0
                + reasoning_tokens * 15.0
                + cache_read_tokens * 0.25) / 1000000.0
        WHEN lower(COALESCE(model, '')) LIKE '%gpt-5.2%'
          OR lower(COALESCE(model, '')) LIKE '%gpt-5.3-codex%'
            THEN (input_tokens * 1.75
                + output_tokens * 14.0
                + reasoning_tokens * 14.0
                + cache_read_tokens * 0.175) / 1000000.0
        WHEN lower(COALESCE(model, '')) LIKE '%gpt-5-codex%'
          OR lower(COALESCE(model, '')) LIKE '%gpt-5.1-codex%'
            THEN (input_tokens * 1.25
                + output_tokens * 10.0
                + reasoning_tokens * 10.0
                + cache_read_tokens * 0.125) / 1000000.0
        WHEN lower(COALESCE(model, '')) LIKE '%codex-mini%'
            THEN (input_tokens * 1.50
                + output_tokens * 6.0
                + reasoning_tokens * 6.0
                + cache_read_tokens * 0.375) / 1000000.0
        WHEN lower(COALESCE(model, '')) LIKE '%codex%'
          OR lower(COALESCE(model, '')) LIKE '%gpt-5%'
            THEN (input_tokens * 1.25
                + output_tokens * 10.0
                + reasoning_tokens * 10.0
                + cache_read_tokens * 0.125) / 1000000.0
        WHEN lower(COALESCE(model, '')) LIKE '%gpt-4%'
            THEN (input_tokens * 2.50
                + output_tokens * 10.0
                + reasoning_tokens * 10.0) / 1000000.0
        ELSE estimated_cost_usd
    END,
    pricing_source = CASE
        WHEN lower(COALESCE(model, '')) LIKE '%opus%'
          OR lower(COALESCE(model, '')) LIKE '%sonnet%'
          OR lower(COALESCE(model, '')) LIKE '%haiku%'
          OR lower(COALESCE(model, '')) LIKE '%gpt-5%'
          OR lower(COALESCE(model, '')) LIKE '%gpt-4%'
          OR lower(COALESCE(model, '')) LIKE '%codex%'
            THEN 'remem_static_backfill'
        ELSE pricing_source
    END
WHERE usage_source = 'text_estimate'
  AND (
      estimated_cost_usd = 0.0
      OR pricing_source IN ('remem_static', 'unknown_pricing')
  );
