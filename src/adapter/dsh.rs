use crate::adapter::{EventSummary, ParsedHookEvent, ToolAdapter};

pub struct DeepSeekHarnessAdapter;

impl ToolAdapter for DeepSeekHarnessAdapter {
    fn name(&self) -> &str {
        "deepseek-harness"
    }

    fn parse_hook(&self, raw_json: &str) -> Option<ParsedHookEvent> {
        let value: serde_json::Value = serde_json::from_str(raw_json).ok()?;
        if value.get("host")?.as_str()? != self.name() {
            return None;
        }
        crate::adapter::common::parse_tool_hook(raw_json)
    }

    fn should_skip(&self, _event: &ParsedHookEvent) -> bool {
        false
    }

    fn should_skip_bash(&self, _command: &str) -> bool {
        false
    }

    fn classify_event(&self, event: &ParsedHookEvent) -> Option<EventSummary> {
        Some(EventSummary {
            event_type: if event
                .tool_input
                .as_ref()
                .and_then(|input| input.get("dsh_event_type"))
                .and_then(serde_json::Value::as_str)
                == Some("assistant/message")
            {
                "message"
            } else {
                "tool_result"
            }
            .into(),
            summary: format!("DSH {}", event.tool_name),
            detail: None,
            files_json: None,
            exit_code: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dsh_preserves_arbitrary_tools_without_claiming_another_host() {
        let adapter = DeepSeekHarnessAdapter;
        let payload = serde_json::json!({
            "host": "deepseek-harness", "session_id": "dsh-1", "cwd": "/tmp",
            "tool_name": "query_records", "tool_input": {"table": "notes"},
            "tool_response": {"rows": ["answer"]}
        });
        let event = adapter.parse_hook(&payload.to_string()).unwrap();
        assert!(!adapter.should_skip(&event));
        assert_eq!(event.tool_name, "query_records");
        assert_eq!(event.tool_response.unwrap()["rows"][0], "answer");
        assert!(adapter.parse_hook(r#"{"session_id":"claude-1"}"#).is_none());
    }

    #[test]
    fn dsh_classifies_native_source_without_tool_name_collisions() {
        let adapter = DeepSeekHarnessAdapter;
        for (source, expected) in [
            (Some("tool/result"), "tool_result"),
            (Some("assistant/message"), "message"),
            (None, "tool_result"),
        ] {
            let payload = serde_json::json!({
                "host": "deepseek-harness", "session_id": "dsh-source", "cwd": "/tmp",
                "tool_name": "assistant/message",
                "tool_input": {
                    "dsh_event_type": source,
                    "arguments": {"dsh_event_type": "assistant/message"}
                },
                "tool_response": {"text": "retained evidence"}
            });
            let event = adapter.parse_hook(&payload.to_string()).unwrap();
            assert_eq!(adapter.classify_event(&event).unwrap().event_type, expected);
        }
    }
}
