use anyhow::Result;
use serde::Deserialize;

use crate::db;

#[derive(Debug, Deserialize)]
struct UserPromptSubmitInput {
    hook_event_name: Option<String>,
    prompt: Option<String>,
}

pub async fn session_init(host: Option<&str>) -> Result<()> {
    let input = std::io::read_to_string(std::io::stdin())?;
    if let Some(output) = session_init_input(&input, host).await? {
        print!("{output}");
    }
    Ok(())
}

async fn session_init_input(input: &str, host: Option<&str>) -> Result<Option<String>> {
    let timer = crate::log::Timer::start("session-init", "");
    if crate::log::debug_enabled() {
        crate::log::debug(
            "session-init",
            &format!(
                "raw input: {}",
                crate::adapter::common::redact_hook_payload_preview(input, 500)
            ),
        );
    }

    let Some((adapter_name, event)) = session_init_event_with_adapter(input, host) else {
        timer.done("skipped");
        return Ok(None);
    };

    let project = event.project.clone();
    let conn = db::open_db_for_hook()?;
    db::upsert_session(&conn, &event.session_id, &event.project, None)?;
    let user_prompt = user_prompt_submit_prompt(input);
    if let Some(prompt) = user_prompt.as_deref() {
        db::record_captured_event(
            &conn,
            &db::CaptureEventInput {
                host: adapter_name,
                session_id: &event.session_id,
                project: &event.project,
                cwd: event.cwd.as_deref(),
                event_type: "user_prompt_submit",
                role: Some("user"),
                tool_name: None,
                content: prompt,
                task_kind: Some(db::ExtractionTaskKind::SessionRollup),
            },
        )?;
    }
    let output = if let Some(prompt) = user_prompt {
        let cwd = event.cwd.as_deref().unwrap_or(&event.project);
        crate::context::prompt_submit_additional_context(
            &conn,
            cwd,
            &event.project,
            &event.session_id,
            &prompt,
            host,
        )?
        .map(|context| user_prompt_submit_output(&context))
        .transpose()?
    } else {
        None
    };

    timer.done(&format!("project={project}"));
    Ok(output)
}

#[cfg(test)]
fn session_init_event(input: &str, host: Option<&str>) -> Option<crate::adapter::ParsedHookEvent> {
    session_init_event_with_adapter(input, host).map(|(_, event)| event)
}

fn session_init_event_with_adapter(
    input: &str,
    host: Option<&str>,
) -> Option<(&'static str, crate::adapter::ParsedHookEvent)> {
    let Some((adapter, event)) = super::hook::detect_adapter_for_host(input, host) else {
        crate::log::warn("session-init", "SKIP no adapter matched hook input");
        return None;
    };

    crate::log::info(
        "session-init",
        &format!("project={} session={}", event.project, event.session_id),
    );
    Some((adapter.name(), event))
}

fn user_prompt_submit_prompt(input: &str) -> Option<String> {
    let hook: UserPromptSubmitInput = serde_json::from_str(input).ok()?;
    let event_name = hook.hook_event_name.as_deref()?.trim();
    if !event_name.eq_ignore_ascii_case("UserPromptSubmit") {
        return None;
    }
    hook.prompt
        .map(|prompt| prompt.trim().to_string())
        .filter(|prompt| !prompt.is_empty())
}

fn user_prompt_submit_output(additional_context: &str) -> Result<String> {
    let output = serde_json::json!({
        "hookSpecificOutput": {
            "hookEventName": "UserPromptSubmit",
            "additionalContext": additional_context
        }
    });
    Ok(serde_json::to_string(&output)?)
}

#[cfg(test)]
mod tests {
    use crate::db::test_support::ScopedTestDataDir;

    use super::{
        session_init_event, session_init_input, user_prompt_submit_output,
        user_prompt_submit_prompt,
    };

    #[test]
    fn session_init_skips_empty_hook_input() {
        let _test_dir = ScopedTestDataDir::new("session-init-empty");

        assert!(session_init_event("", Some("claude-code")).is_none());
    }

    #[test]
    fn session_init_accepts_claude_user_prompt_submit_shape() {
        let _test_dir = ScopedTestDataDir::new("session-init-user-prompt");
        let input = serde_json::json!({
            "session_id": "sess-user-prompt",
            "cwd": "/tmp/remem",
            "hook_event_name": "UserPromptSubmit",
            "prompt": "hello"
        })
        .to_string();

        let Some(event) = session_init_event(&input, Some("claude-code")) else {
            panic!("event should parse");
        };

        assert_eq!(event.session_id, "sess-user-prompt");
        assert_eq!(event.project, "/tmp/remem");
        assert_eq!(user_prompt_submit_prompt(&input).as_deref(), Some("hello"));
    }

    #[tokio::test]
    async fn user_prompt_submit_records_user_captured_event() -> anyhow::Result<()> {
        let test_dir = ScopedTestDataDir::new("session-init-user-prompt-capture");
        std::fs::create_dir_all(&test_dir.path)?;
        let setup = rusqlite::Connection::open(test_dir.db_path())?;
        crate::migrate::run_migrations(&setup)?;
        drop(setup);
        let input = serde_json::json!({
            "session_id": "sess-user-prompt-capture",
            "cwd": "/tmp/remem",
            "hook_event_name": "UserPromptSubmit",
            "prompt": "I prefer concise code reviews."
        })
        .to_string();

        session_init_input(&input, Some("claude-code")).await?;

        let conn = crate::db::open_db()?;
        let (event_type, role, content): (String, Option<String>, Option<String>) = conn
            .query_row(
                "SELECT event_type, role, content_text FROM captured_events",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )?;
        assert_eq!(event_type, "user_prompt_submit");
        assert_eq!(role.as_deref(), Some("user"));
        assert_eq!(content.as_deref(), Some("I prefer concise code reviews."));
        let task_kind: String =
            conn.query_row("SELECT task_kind FROM extraction_tasks", [], |row| {
                row.get(0)
            })?;
        assert_eq!(task_kind, "session_rollup");
        Ok(())
    }

    #[tokio::test]
    async fn session_init_debug_log_redacts_raw_input_before_truncating() -> anyhow::Result<()> {
        let scoped = ScopedTestDataDir::new("session-init-raw-redact");
        unsafe {
            std::env::set_var("REMEM_DEBUG", "1");
            std::env::set_var("REMEM_STDERR_TO_LOG", "1");
        }

        let input = format!(
            r#"{{"authorization":"Bearer ghp_1234567890abcdef","padding":"{}""#,
            "x".repeat(2_000)
        );
        assert!(session_init_input(&input, Some("claude-code"))
            .await?
            .is_none());

        let log = std::fs::read_to_string(scoped.path.join("remem.log"))?;
        assert!(log.contains("[DEBUG]"));
        assert!(
            log.contains("[REDACTED]"),
            "secret should be visibly redacted: {log}"
        );
        assert!(
            !log.contains("ghp_1234567890abcdef"),
            "raw token must not be logged: {log}"
        );

        unsafe {
            std::env::remove_var("REMEM_DEBUG");
            std::env::remove_var("REMEM_STDERR_TO_LOG");
        }
        Ok(())
    }

    #[tokio::test]
    async fn session_init_rejects_stale_schema_without_migrating() -> anyhow::Result<()> {
        let test_dir = ScopedTestDataDir::new("session-init-stale-schema");
        std::fs::create_dir_all(&test_dir.path)?;
        let setup = rusqlite::Connection::open(test_dir.db_path())?;
        setup.execute("CREATE TABLE marker (id INTEGER PRIMARY KEY)", [])?;
        drop(setup);
        let input = serde_json::json!({
            "session_id": "sess-session-init-stale",
            "cwd": "/tmp/remem",
            "hook_event_name": "SessionStart"
        })
        .to_string();

        let err = session_init_input(&input, Some("claude-code"))
            .await
            .expect_err("stale hook database should fail closed");

        assert!(
            err.to_string().contains("hook database open requires"),
            "unexpected error: {err:#}"
        );
        let check = rusqlite::Connection::open(test_dir.db_path())?;
        let migrations_exists: i64 = check.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = '_schema_migrations'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(migrations_exists, 0);
        Ok(())
    }

    #[test]
    fn user_prompt_submit_output_uses_hook_specific_additional_context() -> anyhow::Result<()> {
        let output = user_prompt_submit_output("Remember SQLCipher")?;
        let parsed: serde_json::Value = serde_json::from_str(&output)?;

        assert_eq!(
            parsed["hookSpecificOutput"]["hookEventName"],
            "UserPromptSubmit"
        );
        assert_eq!(
            parsed["hookSpecificOutput"]["additionalContext"],
            "Remember SQLCipher"
        );
        Ok(())
    }
}
