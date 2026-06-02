use serde_json::Value;

use crate::db;
use crate::memory::format::{xml_escape_attr, xml_escape_text};

use super::RollupRange;

const EVENT_CONTENT_LIMIT: usize = 24 * 1024;

pub(super) fn build_rollup_prompt(task: &db::ExtractionTask, range: &RollupRange) -> String {
    let mut prompt = format!(
        "Project: {}\nHost: {}\nSession: {}\nCovered events: {}..{}\n\n",
        task.project,
        task.host,
        task.session_id.as_deref().unwrap_or("<unknown>"),
        range.from_event_id,
        range.to_event_id
    );
    prompt.push_str(
        "Return exactly this XML shape:\n\
         <summary>overall session summary</summary>\n\
         <segments>\n\
         <segment topic_key=\"REPLACE_WITH_TOPIC_KEY\" status=\"open\" confidence=\"0.75\">\n\
         <title>REPLACE_WITH_TITLE</title>\n\
         <summary>REPLACE_WITH_TOPIC_SUMMARY</summary>\n\
         <evidence_event_ids>REPLACE_WITH_EVENT_IDS</evidence_event_ids>\n\
         <from_event_id>REPLACE_WITH_MIN_EVENT_ID</from_event_id>\n\
         <to_event_id>REPLACE_WITH_MAX_EVENT_ID</to_event_id>\n\
         <files>REPLACE_WITH_FILES_OR_EMPTY</files>\n\
         </segment>\n\
         </segments>\n\n\
         Do not copy REPLACE_WITH placeholders; replace every placeholder with facts from the loaded events below.\n\
         topic_key must be stable kebab-case or snake_case.\n\
         status must be one of open, resolved, or superseded.\n\
         evidence_event_ids is authoritative. from_event_id/to_event_id must be min/max evidence IDs.\n\
         If there are no coherent topic segments, return an empty <segments></segments>.\n\n",
    );

    let mut previous_epoch: Option<i64> = None;
    for event in &range.events {
        let gap_before = previous_epoch.map(|epoch| (event.created_at_epoch - epoch).max(0));
        previous_epoch = Some(event.created_at_epoch);
        let files_touched = files_touched_for_prompt(&event.content);

        prompt.push_str(&format!(
            "<event id=\"{}\" type=\"{}\" created_at_epoch=\"{}\" tokens=\"{}\"",
            event.id,
            xml_escape_attr(&event.event_type),
            event.created_at_epoch,
            event.token_estimate
        ));
        if let Some(gap_before) = gap_before {
            prompt.push_str(&format!(" gap_before=\"{}\"", gap_before));
        }
        if let Some(turn_id) = event.turn_id.as_deref() {
            prompt.push_str(&format!(" turn_id=\"{}\"", xml_escape_attr(turn_id)));
        }
        if let Some(role) = event.role.as_deref() {
            prompt.push_str(&format!(" role=\"{}\"", xml_escape_attr(role)));
        }
        if let Some(tool_name) = event.tool_name.as_deref() {
            prompt.push_str(&format!(" tool=\"{}\"", xml_escape_attr(tool_name)));
        }
        if !files_touched.is_empty() {
            prompt.push_str(&format!(
                " files_touched=\"{}\"",
                xml_escape_attr(&files_touched.join(","))
            ));
        }
        prompt.push_str(">\n");
        prompt.push_str(&xml_escape_text(db::truncate_str(
            &event.content,
            EVENT_CONTENT_LIMIT,
        )));
        prompt.push_str("\n</event>\n\n");
    }
    prompt
}

fn files_touched_for_prompt(content: &str) -> Vec<String> {
    let Ok(value) = serde_json::from_str::<Value>(content) else {
        return Vec::new();
    };
    let mut files = Vec::new();
    collect_file_values(&value, None, &mut files);
    files.sort();
    files.dedup();
    files.truncate(12);
    files
}

fn collect_file_values(value: &Value, key: Option<&str>, out: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            for (child_key, child_value) in map {
                collect_file_values(child_value, Some(child_key), out);
            }
        }
        Value::Array(values) => {
            for child in values {
                collect_file_values(child, key, out);
            }
        }
        Value::String(raw) if key.is_some_and(is_file_key) && looks_like_file_path(raw) => {
            out.push(raw.to_string());
        }
        _ => {}
    }
}

fn is_file_key(key: &str) -> bool {
    matches!(
        key,
        "file" | "files" | "file_path" | "file_paths" | "notebook_path" | "path"
    )
}

fn looks_like_file_path(value: &str) -> bool {
    let trimmed = value.trim();
    !trimmed.is_empty()
        && trimmed.len() <= 240
        && !trimmed.contains('\n')
        && !trimmed.starts_with("http://")
        && !trimmed.starts_with("https://")
        && (trimmed.contains('/') || trimmed.contains('.'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::ExtractionTaskKind;

    #[test]
    fn files_touched_uses_structured_json_fields() {
        let files = files_touched_for_prompt(
            r#"{"command":"cat src/lib.rs","file_path":"src/lib.rs","url":"https://example.test"}"#,
        );
        assert_eq!(files, vec!["src/lib.rs"]);
    }

    #[test]
    fn rollup_prompt_placeholders_are_not_parseable_literals() {
        let task = db::ExtractionTask {
            id: 1,
            task_kind: ExtractionTaskKind::SessionRollup,
            host_id: 1,
            workspace_id: 1,
            project_id: 1,
            session_row_id: Some(1),
            host: "codex-cli".to_string(),
            project: "/repo".to_string(),
            session_id: Some("session-1".to_string()),
            priority: 0,
            cursor_event_id: Some(0),
            high_watermark_event_id: Some(3),
            attempts: 0,
        };
        let range = RollupRange {
            from_event_id: 1,
            to_event_id: 3,
            events: vec![super::super::RollupEvent {
                id: 1,
                event_type: "tool_result".to_string(),
                role: None,
                tool_name: None,
                content: "first event".to_string(),
                token_estimate: 1,
                created_at_epoch: 100,
                turn_id: None,
            }],
        };

        let prompt = build_rollup_prompt(&task, &range);

        assert!(prompt.contains("topic_key=\"REPLACE_WITH_TOPIC_KEY\""));
        assert!(prompt.contains("<evidence_event_ids>REPLACE_WITH_EVENT_IDS</evidence_event_ids>"));
        assert!(prompt.contains("Do not copy REPLACE_WITH placeholders"));
        assert!(!prompt.contains("topic_key=\"stable-kebab-case\""));
        assert!(!prompt.contains("<evidence_event_ids>1,2,3</evidence_event_ids>"));
    }
}
