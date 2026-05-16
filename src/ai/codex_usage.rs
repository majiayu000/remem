use std::io::{BufRead, BufReader};

use anyhow::Result;
use serde_json::Value;

use crate::ai::types::TokenUsage;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CodexRunUsage {
    pub usage: TokenUsage,
    pub model: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct CodexCounters {
    input_tokens: i64,
    cached_input_tokens: i64,
    output_tokens: i64,
    reasoning_output_tokens: i64,
}

impl CodexCounters {
    fn from_value(value: &Value) -> Self {
        Self {
            input_tokens: json_i64(value, "input_tokens"),
            cached_input_tokens: json_i64(value, "cached_input_tokens")
                .max(json_i64(value, "cache_read_input_tokens")),
            output_tokens: json_i64(value, "output_tokens"),
            reasoning_output_tokens: json_i64(value, "reasoning_output_tokens"),
        }
    }

    fn is_empty(self) -> bool {
        self.input_tokens == 0
            && self.cached_input_tokens == 0
            && self.output_tokens == 0
            && self.reasoning_output_tokens == 0
    }

    fn into_token_usage(self) -> TokenUsage {
        let input_tokens = (self.input_tokens - self.cached_input_tokens).max(0);
        let output_tokens = (self.output_tokens - self.reasoning_output_tokens).max(0);
        TokenUsage {
            input_tokens,
            output_tokens,
            reasoning_tokens: self.reasoning_output_tokens,
            cache_read_tokens: self.cached_input_tokens,
            raw_input_tokens: self.input_tokens,
            raw_output_tokens: self.output_tokens,
            ..TokenUsage::default()
        }
    }
}

pub(super) fn parse_codex_json_events(
    events: &[u8],
    model: Option<String>,
) -> Result<Option<CodexRunUsage>> {
    let mut usage = TokenUsage::default();
    let reader = BufReader::new(events);

    for line in reader.lines() {
        let line = line?;
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if value.get("type").and_then(Value::as_str) != Some("turn.completed") {
            continue;
        }
        let Some(raw_usage) = value.get("usage") else {
            continue;
        };
        let counters = CodexCounters::from_value(raw_usage);
        if counters.is_empty() {
            continue;
        }
        usage.add(&counters.into_token_usage());
    }

    if usage.is_empty() {
        Ok(None)
    } else {
        Ok(Some(CodexRunUsage { usage, model }))
    }
}

fn json_i64(value: &Value, key: &str) -> i64 {
    value.get(key).and_then(Value::as_i64).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use anyhow::{Context, Result};

    use super::parse_codex_json_events;

    #[test]
    fn parses_codex_exec_json_turn_completed_usage() -> Result<()> {
        let log = r#"
{"type":"thread.started","thread_id":"019e2f80-913f-7452-97ed-6340c56b2bd4"}
{"type":"turn.started"}
{"type":"item.completed","item":{"id":"item_0","type":"agent_message","text":"OK"}}
{"type":"turn.completed","usage":{"input_tokens":1000,"cached_input_tokens":200,"output_tokens":500,"reasoning_output_tokens":150}}
"#;
        let parsed = parse_codex_json_events(log.as_bytes(), Some("gpt-5.2".to_string()))
            .and_then(|usage| usage.context("usage should parse"))?;
        assert_eq!(parsed.model.as_deref(), Some("gpt-5.2"));
        assert_eq!(parsed.usage.input_tokens, 800);
        assert_eq!(parsed.usage.cache_read_tokens, 200);
        assert_eq!(parsed.usage.output_tokens, 350);
        assert_eq!(parsed.usage.reasoning_tokens, 150);
        assert_eq!(parsed.usage.total_tokens(), 1500);
        Ok(())
    }

    #[test]
    fn parses_multiple_codex_exec_json_turns() -> Result<()> {
        let log = r#"
{"type":"turn.completed","usage":{"input_tokens":100,"cached_input_tokens":80,"output_tokens":20,"reasoning_output_tokens":5}}
{"type":"turn.completed","usage":{"input_tokens":50,"cached_input_tokens":0,"output_tokens":10,"reasoning_output_tokens":0}}
"#;
        let parsed = parse_codex_json_events(log.as_bytes(), None)
            .and_then(|usage| usage.context("usage should parse"))?;
        assert_eq!(parsed.usage.input_tokens, 70);
        assert_eq!(parsed.usage.cache_read_tokens, 80);
        assert_eq!(parsed.usage.output_tokens, 25);
        assert_eq!(parsed.usage.reasoning_tokens, 5);
        assert_eq!(parsed.usage.total_tokens(), 180);
        Ok(())
    }

    #[test]
    fn returns_none_without_turn_completed_usage() -> Result<()> {
        let log = r#"
{"type":"thread.started","thread_id":"019e2f80-913f-7452-97ed-6340c56b2bd4"}
{"type":"item.completed","item":{"id":"item_0","type":"agent_message","text":"OK"}}
"#;
        let parsed = parse_codex_json_events(log.as_bytes(), Some("gpt-5.2".to_string()))?;
        assert!(parsed.is_none());
        Ok(())
    }
}
