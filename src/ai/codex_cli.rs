use std::ffi::OsString;
use std::path::Path;

use anyhow::{Context, Result};
use tokio::process::Command;

use crate::ai::config::{get_codex_model, get_codex_path};
use crate::ai::types::{AiCallResult, AI_TIMEOUT_SECS};

pub(super) async fn call_codex_cli(system: &str, user_message: &str) -> Result<AiCallResult> {
    let codex = get_codex_path();
    let model = get_codex_model();
    let output_path = std::env::temp_dir().join(format!(
        "remem-codex-summary-{}-{}.txt",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    let prompt = build_prompt(system, user_message);

    let mut command = Command::new(&codex);
    command.args(build_codex_args(&output_path, model.as_deref()));
    command
        .env("REMEM_DISABLE_HOOKS", "1")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);

    let mut child = command
        .spawn()
        .with_context(|| format!("failed to spawn '{}' - is Codex CLI installed?", codex))?;

    if let Some(mut stdin) = child.stdin.take() {
        use tokio::io::AsyncWriteExt;
        stdin.write_all(prompt.as_bytes()).await?;
    }

    let output = tokio::time::timeout(
        std::time::Duration::from_secs(AI_TIMEOUT_SECS),
        child.wait_with_output(),
    )
    .await
    .map_err(|_| anyhow::anyhow!("codex CLI timed out after {}s", AI_TIMEOUT_SECS))??;

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

    Ok(AiCallResult {
        text,
        executor: "codex-cli",
        model: model.unwrap_or_else(|| "codex-default".to_string()),
    })
}

fn build_codex_args(output_path: &Path, model: Option<&str>) -> Vec<OsString> {
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
        let args = build_codex_args(Path::new("/tmp/remem-out.txt"), Some("gpt-test"));
        let rendered: Vec<String> = args
            .iter()
            .map(|arg| arg.to_string_lossy().to_string())
            .collect();

        assert_eq!(&rendered[..3], ["--ask-for-approval", "never", "exec"]);
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
        assert_eq!(rendered.last().map(String::as_str), Some("-"));
    }
}
