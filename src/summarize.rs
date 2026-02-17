use anyhow::Result;
use serde::Deserialize;

use crate::db;
use crate::observe;

const SUMMARY_PROMPT: &str = include_str!("../prompts/summary.txt");

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SummarizeInput {
    session_id: Option<String>,
    cwd: Option<String>,
    transcript_path: Option<String>,
    last_assistant_message: Option<String>,
}

fn project_from_cwd(cwd: &str) -> String {
    std::path::Path::new(cwd)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| cwd.to_string())
}

fn extract_last_assistant_message(transcript_path: &str) -> Option<String> {
    let content = std::fs::read_to_string(transcript_path).ok()?;
    let mut last_assistant = None;

    for line in content.lines().rev() {
        let val: serde_json::Value = serde_json::from_str(line).ok()?;
        if val["type"].as_str() == Some("assistant") {
            let text_parts: Vec<&str> = val["message"]["content"]
                .as_array()?
                .iter()
                .filter_map(|c| {
                    if c["type"].as_str() == Some("text") {
                        c["text"].as_str()
                    } else {
                        None
                    }
                })
                .collect();
            if !text_parts.is_empty() {
                last_assistant = Some(text_parts.join("\n"));
                break;
            }
        }
    }
    last_assistant
}

pub struct ParsedSummary {
    pub request: Option<String>,
    pub investigated: Option<String>,
    pub learned: Option<String>,
    pub completed: Option<String>,
    pub next_steps: Option<String>,
    pub notes: Option<String>,
}

pub fn parse_summary(text: &str) -> Option<ParsedSummary> {
    // Check for skip
    if text.contains("<skip_summary") {
        return None;
    }

    let start = text.find("<summary>")?;
    let end = text.find("</summary>")?;
    let content = &text[start + "<summary>".len()..end];

    Some(ParsedSummary {
        request: observe::extract_field(content, "request"),
        investigated: observe::extract_field(content, "investigated"),
        learned: observe::extract_field(content, "learned"),
        completed: observe::extract_field(content, "completed"),
        next_steps: observe::extract_field(content, "next_steps"),
        notes: observe::extract_field(content, "notes"),
    })
}

pub async fn summarize() -> Result<()> {
    let timer = crate::log::Timer::start("summarize", "");
    let input = std::io::read_to_string(std::io::stdin())?;
    let hook: SummarizeInput = serde_json::from_str(&input)?;

    let session_id = hook
        .session_id
        .ok_or_else(|| anyhow::anyhow!("missing session_id"))?;
    let cwd = hook.cwd.as_deref().unwrap_or(".");
    let project = project_from_cwd(cwd);

    crate::log::info("summarize", &format!("project={} session={}", project, session_id));

    // Flush pending observation queue before summarizing
    match observe::flush_pending(&session_id, &project).await {
        Ok(n) => {
            if n > 0 {
                crate::log::info("summarize", &format!("flushed {} observations from queue", n));
            }
        }
        Err(e) => {
            crate::log::warn("summarize", &format!("flush failed (continuing): {}", e));
        }
    }

    // Get last assistant message
    let assistant_msg = hook
        .last_assistant_message
        .or_else(|| {
            hook.transcript_path
                .as_deref()
                .and_then(extract_last_assistant_message)
        })
        .unwrap_or_default();

    if assistant_msg.is_empty() {
        crate::log::warn("summarize", "no assistant message, skipping");
        timer.done("no message");
        return Ok(());
    }

    crate::log::info("summarize", &format!("message len={}B", assistant_msg.len()));

    // Truncate if too long
    let msg = if assistant_msg.len() > 12000 {
        assistant_msg[..12000].to_string()
    } else {
        assistant_msg
    };

    let user_message = format!(
        "Here is the assistant's last response from the session:\n\n{}",
        msg
    );

    let ai_start = std::time::Instant::now();
    let response = observe::call_anthropic(SUMMARY_PROMPT, &user_message).await?;
    let ai_ms = ai_start.elapsed().as_millis();
    crate::log::info("summarize", &format!("AI response {}ms {}B", ai_ms, response.len()));

    let Some(summary) = parse_summary(&response) else {
        crate::log::info("summarize", "session skipped (trivial)");
        timer.done("skipped");
        return Ok(());
    };

    let conn = db::open_db()?;
    let memory_session_id = db::upsert_session(&conn, &session_id, &project, None)?;

    let usage = response.len() as i64 / 4;
    db::insert_summary(
        &conn,
        &memory_session_id,
        &project,
        summary.request.as_deref(),
        summary.investigated.as_deref(),
        summary.learned.as_deref(),
        summary.completed.as_deref(),
        summary.next_steps.as_deref(),
        summary.notes.as_deref(),
        None,
        usage,
    )?;

    let request_preview = summary.request.as_deref().unwrap_or("-");
    timer.done(&format!("~{}tok request=\"{}\"", usage, request_preview));
    Ok(())
}
