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
    crate::log::debug(
        "session-init",
        &format!("raw input: {}", crate::db::truncate_str(input, 500)),
    );

    let Some(event) = session_init_event(input, host) else {
        timer.done("skipped");
        return Ok(None);
    };

    let project = event.project.clone();
    let conn = db::open_db()?;
    db::upsert_session(&conn, &event.session_id, &event.project, None)?;
    let output = if let Some(prompt) = user_prompt_submit_prompt(input) {
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

fn session_init_event(input: &str, host: Option<&str>) -> Option<crate::adapter::ParsedHookEvent> {
    let Some((_adapter, event)) = super::hook::detect_adapter_for_host(input, host) else {
        crate::log::warn("session-init", "SKIP no adapter matched hook input");
        return None;
    };

    crate::log::info(
        "session-init",
        &format!("project={} session={}", event.project, event.session_id),
    );
    Some(event)
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

    use super::{session_init_event, user_prompt_submit_output, user_prompt_submit_prompt};

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
