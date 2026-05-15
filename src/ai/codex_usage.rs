use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use anyhow::{Context, Result};
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
    total_tokens: i64,
}

impl CodexCounters {
    fn from_value(value: &Value) -> Self {
        Self {
            input_tokens: json_i64(value, "input_tokens"),
            cached_input_tokens: json_i64(value, "cached_input_tokens")
                .max(json_i64(value, "cache_read_input_tokens")),
            output_tokens: json_i64(value, "output_tokens"),
            reasoning_output_tokens: json_i64(value, "reasoning_output_tokens"),
            total_tokens: json_i64(value, "total_tokens"),
        }
    }

    fn subtract(self, previous: Self) -> Self {
        Self {
            input_tokens: (self.input_tokens - previous.input_tokens).max(0),
            cached_input_tokens: (self.cached_input_tokens - previous.cached_input_tokens).max(0),
            output_tokens: (self.output_tokens - previous.output_tokens).max(0),
            reasoning_output_tokens: (self.reasoning_output_tokens
                - previous.reasoning_output_tokens)
                .max(0),
            total_tokens: (self.total_tokens - previous.total_tokens).max(0),
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

pub(super) fn collect_codex_usage_for_run(
    run_id: &str,
    started_at: SystemTime,
) -> Result<Option<CodexRunUsage>> {
    let Some(sessions_dir) = codex_sessions_dir() else {
        return Ok(None);
    };
    let threshold = started_at
        .checked_sub(Duration::from_secs(60))
        .unwrap_or(SystemTime::UNIX_EPOCH);

    let mut files = Vec::new();
    collect_recent_jsonl_files(&sessions_dir, threshold, &mut files)?;

    let mut aggregate = TokenUsage::default();
    let mut model = None;
    let mut matched_any = false;
    for path in files {
        let Some(run_usage) = parse_codex_file(&path, run_id)? else {
            continue;
        };
        matched_any = true;
        aggregate.add(&run_usage.usage);
        if run_usage.model.is_some() {
            model = run_usage.model;
        }
    }

    if matched_any && !aggregate.is_empty() {
        Ok(Some(CodexRunUsage {
            usage: aggregate,
            model,
        }))
    } else {
        Ok(None)
    }
}

fn codex_sessions_dir() -> Option<PathBuf> {
    if let Ok(codex_home) = std::env::var("CODEX_HOME") {
        let path = PathBuf::from(codex_home).join("sessions");
        if path.is_dir() {
            return Some(path);
        }
    }

    let home = dirs::home_dir()?;
    let path = home.join(".codex").join("sessions");
    path.is_dir().then_some(path)
}

fn collect_recent_jsonl_files(
    dir: &Path,
    threshold: SystemTime,
    out: &mut Vec<PathBuf>,
) -> Result<()> {
    for entry in fs::read_dir(dir).with_context(|| format!("read {}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_recent_jsonl_files(&path, threshold, out)?;
            continue;
        }
        if path.extension().and_then(|ext| ext.to_str()) != Some("jsonl") {
            continue;
        }
        let modified = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        if modified >= threshold {
            out.push(path);
        }
    }
    Ok(())
}

fn parse_codex_file(path: &Path, run_id: &str) -> Result<Option<CodexRunUsage>> {
    let file = File::open(path).with_context(|| format!("open {}", path.display()))?;
    parse_codex_reader(BufReader::new(file), run_id)
}

fn parse_codex_reader(reader: impl BufRead, run_id: &str) -> Result<Option<CodexRunUsage>> {
    let mut saw_run_id = false;
    let mut previous_total: Option<CodexCounters> = None;
    let mut current_model: Option<String> = None;
    let mut usage = TokenUsage::default();

    for line in reader.lines() {
        let line = line?;
        if line.contains(run_id) {
            saw_run_id = true;
        }
        if !saw_run_id {
            continue;
        }

        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let entry_type = value.get("type").and_then(Value::as_str);
        let Some(payload) = value.get("payload") else {
            continue;
        };

        if entry_type == Some("turn_context") {
            if let Some(model) = extract_model(payload) {
                current_model = Some(model);
            }
            continue;
        }

        if entry_type != Some("event_msg")
            || payload.get("type").and_then(Value::as_str) != Some("token_count")
        {
            continue;
        }

        let Some(info) = payload.get("info") else {
            continue;
        };
        if let Some(model) = extract_model(payload) {
            current_model = Some(model);
        }

        let Some(total_value) = info.get("total_token_usage") else {
            continue;
        };
        let total = CodexCounters::from_value(total_value);
        if previous_total.is_some_and(|previous| total.total_tokens == previous.total_tokens) {
            continue;
        }

        let delta = info
            .get("last_token_usage")
            .map(CodexCounters::from_value)
            .filter(|counters| !counters.is_empty())
            .unwrap_or_else(|| previous_total.map_or(total, |previous| total.subtract(previous)));
        previous_total = Some(total);
        if delta.is_empty() {
            continue;
        }
        usage.add(&delta.into_token_usage());
    }

    if saw_run_id {
        Ok(Some(CodexRunUsage {
            usage,
            model: current_model,
        }))
    } else {
        Ok(None)
    }
}

fn extract_model(payload: &Value) -> Option<String> {
    let info = payload.get("info");
    [
        info.and_then(|value| value.get("model")),
        info.and_then(|value| value.get("model_name")),
        info.and_then(|value| value.get("metadata"))
            .and_then(|metadata| metadata.get("model")),
        payload.get("model"),
    ]
    .into_iter()
    .flatten()
    .filter_map(Value::as_str)
    .map(str::trim)
    .find(|model| !model.is_empty())
    .map(ToOwned::to_owned)
}

fn json_i64(value: &Value, key: &str) -> i64 {
    value.get(key).and_then(Value::as_i64).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::parse_codex_reader;

    #[test]
    fn parses_last_token_usage_after_run_marker() {
        let log = r#"
{"type":"response_item","payload":{"item":{"type":"message","content":[{"text":"run remem-usage-1"}]}}}
{"type":"event_msg","payload":{"type":"token_count","info":{"model":"gpt-5.5","total_token_usage":{"input_tokens":1000,"cached_input_tokens":200,"output_tokens":500,"reasoning_output_tokens":150,"total_tokens":1500},"last_token_usage":{"input_tokens":1000,"cached_input_tokens":200,"output_tokens":500,"reasoning_output_tokens":150,"total_tokens":1500}}}}
"#;
        let parsed = parse_codex_reader(Cursor::new(log), "remem-usage-1")
            .expect("parse should succeed")
            .expect("run should match");
        assert_eq!(parsed.model.as_deref(), Some("gpt-5.5"));
        assert_eq!(parsed.usage.input_tokens, 800);
        assert_eq!(parsed.usage.cache_read_tokens, 200);
        assert_eq!(parsed.usage.output_tokens, 350);
        assert_eq!(parsed.usage.reasoning_tokens, 150);
        assert_eq!(parsed.usage.total_tokens(), 1500);
    }

    #[test]
    fn computes_delta_when_last_usage_is_missing() {
        let log = r#"
{"type":"response_item","payload":{"item":{"type":"message","content":[{"text":"run remem-usage-2"}]}}}
{"type":"event_msg","payload":{"type":"token_count","info":{"model":"gpt-5.4","total_token_usage":{"input_tokens":100,"cached_input_tokens":0,"output_tokens":50,"reasoning_output_tokens":10,"total_tokens":150}}}}
{"type":"event_msg","payload":{"type":"token_count","info":{"model":"gpt-5.4","total_token_usage":{"input_tokens":400,"cached_input_tokens":100,"output_tokens":150,"reasoning_output_tokens":40,"total_tokens":550}}}}
"#;
        let parsed = parse_codex_reader(Cursor::new(log), "remem-usage-2")
            .expect("parse should succeed")
            .expect("run should match");
        assert_eq!(parsed.usage.input_tokens, 300);
        assert_eq!(parsed.usage.cache_read_tokens, 100);
        assert_eq!(parsed.usage.output_tokens, 110);
        assert_eq!(parsed.usage.reasoning_tokens, 40);
        assert_eq!(parsed.usage.total_tokens(), 550);
    }

    #[test]
    fn ignores_files_without_run_marker() {
        let log = r#"{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":100,"output_tokens":50,"total_tokens":150}}}}"#;
        let parsed = parse_codex_reader(Cursor::new(log), "missing").expect("parse should succeed");
        assert!(parsed.is_none());
    }
}
