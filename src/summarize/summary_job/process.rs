use anyhow::Result;

use crate::db;
use crate::db::project_from_cwd;

use super::super::constants::{
    SUMMARIZE_COOLDOWN_SECS, SUMMARIZE_LOCK_TIMEOUT_SECS, SUMMARY_PROMPT,
};
use super::super::input::{extract_last_assistant_message, hash_message, SummarizeInput};
use super::super::parse::parse_summary;
use super::persist::{build_existing_summary_context, finalize_summary, sync_native_memory};

pub async fn process_summary_job_input(input: &str) -> Result<()> {
    let hook: SummarizeInput = serde_json::from_str(input)?;
    let Some(session_id) = hook.session_id else {
        return Ok(());
    };
    let cwd = hook.cwd.as_deref().unwrap_or(".");
    let project = project_from_cwd(cwd);

    let assistant_msg = hook
        .last_assistant_message
        .or_else(|| {
            hook.transcript_path
                .as_deref()
                .and_then(extract_last_assistant_message)
        })
        .unwrap_or_default();
    let Some(msg) = prepare_assistant_message(assistant_msg) else {
        return Ok(());
    };

    let mut conn = db::open_db()?;
    if db::is_summarize_on_cooldown(&conn, &project, SUMMARIZE_COOLDOWN_SECS)? {
        crate::log::info(
            "summary-job",
            &format!("project={} on cooldown, skipping", project),
        );
        return Ok(());
    }

    let msg_hash = hash_message(&msg);
    if db::is_duplicate_message(&conn, &project, &msg_hash)? {
        crate::log::info(
            "summary-job",
            &format!("project={} duplicate message, skipping", project),
        );
        return Ok(());
    }

    let memory_sid = db::upsert_session(&conn, &session_id, &project, None)?;
    let existing_ctx = build_existing_summary_context(&conn, &memory_sid, &project)?;
    let user_message = format!(
        "{}Here is the assistant's last response from the session:\n\n{}",
        existing_ctx, msg
    );

    if !db::try_acquire_summarize_lock(&mut conn, &project, SUMMARIZE_LOCK_TIMEOUT_SECS)? {
        crate::log::info(
            "summary-job",
            &format!("project={} summarize lock held, skipping", project),
        );
        return Ok(());
    }

    let response = call_summary_ai(&project, &user_message)
        .await
        .map_err(|err| {
            let _ = db::release_summarize_lock(&conn, &project);
            anyhow::anyhow!("summary ai failed: {}", err)
        })?;
    let Some(summary) = parse_summary(&response) else {
        let _ = db::release_summarize_lock(&conn, &project);
        crate::log::info("summary-job", "session skipped by AI");
        return Ok(());
    };

    finalize_summary(
        &mut conn,
        &session_id,
        &memory_sid,
        &project,
        &msg_hash,
        summary,
    )?;
    sync_native_memory(cwd, &project);
    Ok(())
}

fn prepare_assistant_message(message: String) -> Option<String> {
    if message.is_empty() || message.contains("<skip_summary") || message.len() < 50 {
        return None;
    }
    if message.len() > 12000 {
        Some(crate::db::truncate_str(&message, 12000).to_string())
    } else {
        Some(message)
    }
}

async fn call_summary_ai(project: &str, user_message: &str) -> Result<String> {
    let ai_start = std::time::Instant::now();
    let response = crate::ai::call_ai(
        SUMMARY_PROMPT,
        user_message,
        crate::ai::UsageContext {
            project: Some(project),
            operation: "summarize",
        },
    )
    .await?;
    crate::log::info(
        "summary-job",
        &format!(
            "AI response {}ms {}B",
            ai_start.elapsed().as_millis(),
            response.len()
        ),
    );
    Ok(response)
}
