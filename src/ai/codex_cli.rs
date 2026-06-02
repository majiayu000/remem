use std::ffi::OsString;
use std::path::Path;

use anyhow::{Context, Result};
use tokio::process::Command;

use crate::ai::config::{get_codex_model, get_codex_path, get_codex_reasoning_effort};
use crate::ai::types::{AiCallResult, AI_TIMEOUT_SECS};

pub(super) async fn call_codex_cli(system: &str, user_message: &str) -> Result<AiCallResult> {
    let codex = get_codex_path();
    let model = get_codex_model();
    let reasoning_effort = get_codex_reasoning_effort();
    let output_path = std::env::temp_dir().join(format!(
        "remem-codex-summary-{}-{}.txt",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    let prompt = build_prompt(system, user_message);
    let working_dir = super::stable_working_dir();

    let mut command = Command::new(&codex);
    command.args(build_codex_args(
        &output_path,
        model.as_deref(),
        reasoning_effort.as_deref(),
    ));
    command
        .current_dir(&working_dir)
        .env("REMEM_DISABLE_HOOKS", "1")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);

    let mut child = command
        .spawn()
        .with_context(|| format!("failed to spawn '{}' - is Codex CLI installed?", codex))?;

    if let Some(mut stdin) = child.stdin.take() {
        use tokio::io::AsyncWriteExt;
        stdin.write_all(prompt.as_bytes()).await?;
    }

    let output = match tokio::time::timeout(
        std::time::Duration::from_secs(AI_TIMEOUT_SECS),
        child.wait_with_output(),
    )
    .await
    {
        Ok(Ok(output)) => output,
        Ok(Err(error)) => {
            let _ = std::fs::remove_file(&output_path);
            return Err(error.into());
        }
        Err(_) => {
            let _ = std::fs::remove_file(&output_path);
            anyhow::bail!("codex CLI timed out after {}s", AI_TIMEOUT_SECS);
        }
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let _ = std::fs::remove_file(&output_path);
        anyhow::bail!("codex CLI exited {}: {}", output.status, stderr);
    }

    let text = std::fs::read_to_string(&output_path)
        .with_context(|| format!("failed to read Codex output {}", output_path.display()))?
        .trim()
        .to_string();
    let _ = std::fs::remove_file(&output_path);
    if text.is_empty() {
        anyhow::bail!("codex CLI returned empty response");
    }

    let codex_usage =
        match super::codex_usage::parse_codex_json_events(&output.stdout, model.clone()) {
            Ok(usage) => usage,
            Err(error) => {
                crate::log::warn("ai", &format!("codex usage parse failed: {}", error));
                None
            }
        };
    if codex_usage.is_none() {
        crate::log::warn("ai", "codex JSON usage parse found no turn.completed usage");
    }
    let usage = codex_usage
        .as_ref()
        .map(|run_usage| run_usage.usage.clone());
    let usage_model = codex_usage.and_then(|run_usage| run_usage.model);

    Ok(AiCallResult {
        text,
        executor: "codex-cli",
        model: usage_model
            .or(model)
            .unwrap_or_else(|| "codex-default".to_string()),
        usage,
        usage_source: Some("codex_log"),
    })
}

fn build_codex_args(
    output_path: &Path,
    model: Option<&str>,
    reasoning_effort: Option<&str>,
) -> Vec<OsString> {
    let mut args: Vec<OsString> = [
        "--ask-for-approval",
        "never",
        "exec",
        "--ephemeral",
        "--ignore-user-config",
        "--ignore-rules",
        "--skip-git-repo-check",
        "--sandbox",
        "read-only",
        "--json",
        "--output-last-message",
    ]
    .into_iter()
    .map(OsString::from)
    .collect();

    args.push(output_path.as_os_str().to_owned());
    if let Some(model) = model {
        args.push(OsString::from("--model"));
        args.push(OsString::from(model));
    }
    if let Some(reasoning_effort) = reasoning_effort {
        args.push(OsString::from("-c"));
        args.push(OsString::from(format!(
            "model_reasoning_effort=\"{}\"",
            reasoning_effort
        )));
    }
    args.push(OsString::from("-"));
    args
}

fn build_prompt(system: &str, user_message: &str) -> String {
    format!(
        "You are running as remem's Codex CLI summarization backend.\n\
         Follow the system instructions exactly and return only the requested output.\n\n\
         <system>\n{}\n</system>\n\n\
         <input>\n{}\n</input>\n",
        system, user_message
    )
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::build_codex_args;

    #[test]
    fn codex_args_put_global_approval_before_exec() {
        let args = build_codex_args(
            Path::new("/tmp/remem-out.txt"),
            Some("gpt-test"),
            Some("low"),
        );
        let rendered: Vec<String> = args
            .iter()
            .map(|arg| arg.to_string_lossy().to_string())
            .collect();

        assert_eq!(&rendered[..3], ["--ask-for-approval", "never", "exec"]);
        for isolation_arg in ["--ephemeral", "--ignore-user-config", "--ignore-rules"] {
            assert!(
                rendered.iter().any(|arg| arg == isolation_arg),
                "{rendered:?}"
            );
        }
        assert!(rendered.iter().any(|arg| arg == "--json"), "{rendered:?}");
        assert!(
            rendered
                .windows(2)
                .any(|pair| pair[0] == "--output-last-message" && pair[1] == "/tmp/remem-out.txt"),
            "{rendered:?}"
        );
        assert!(
            rendered
                .windows(2)
                .any(|pair| pair[0] == "--model" && pair[1] == "gpt-test"),
            "{rendered:?}"
        );
        assert!(
            rendered
                .windows(2)
                .any(|pair| pair[0] == "-c" && pair[1] == "model_reasoning_effort=\"low\""),
            "{rendered:?}"
        );
        assert_eq!(rendered.last().map(String::as_str), Some("-"));
    }
}
