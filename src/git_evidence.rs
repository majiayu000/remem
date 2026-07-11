use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use regex::Regex;
use serde_json::Value;

use crate::adapter::{EventSummary, ParsedHookEvent};
use crate::git_util::{GitCommitEvidence, GitEvidenceKind};

pub(crate) fn from_observed_event(
    event: &ParsedHookEvent,
    summary: &EventSummary,
) -> Result<Vec<GitCommitEvidence>> {
    if event.tool_name != "Bash" || summary.exit_code.is_some_and(|code| code != 0) {
        return Ok(Vec::new());
    }
    let Some(command) = event
        .tool_input
        .as_ref()
        .and_then(|value| value.get("command"))
        .and_then(Value::as_str)
    else {
        return Ok(Vec::new());
    };
    let Some(base_cwd) = event.cwd.as_deref() else {
        return Ok(Vec::new());
    };
    let Some(cwd) = commit_workdir(command, base_cwd) else {
        return Ok(Vec::new());
    };
    let output = hook_response_output(event.tool_response.as_ref());
    let Some(candidate) = commit_candidate_from_output(&output)? else {
        return Ok(Vec::new());
    };
    let cwd = cwd.to_string_lossy();
    Ok(vec![GitCommitEvidence {
        kind: GitEvidenceKind::ObservedCommit,
        metadata: crate::git_util::resolve_commit_metadata(&cwd, &candidate).with_context(
            || format!("resolve successful git commit candidate {candidate} from hook event"),
        )?,
        locator: Some("post_tool_use".to_string()),
    }])
}

pub(crate) fn from_codex_transcript(
    transcript_path: &str,
    byte_limit: u64,
    fallback_cwd: &str,
) -> Result<Vec<GitCommitEvidence>> {
    let content =
        crate::memory::raw_transcript::read_transcript_content(transcript_path, Some(byte_limit))
            .with_context(|| {
            format!(
                "read Codex transcript commit evidence path={transcript_path} bytes={byte_limit}"
            )
        })?;
    let mut calls = BTreeMap::<String, CommitCall>::new();
    let mut evidence = BTreeMap::<String, GitCommitEvidence>::new();
    for (line_index, line) in content.lines().enumerate() {
        let value: Value = match serde_json::from_str(line) {
            Ok(value) => value,
            Err(_) if !line.contains("\"response_item\"") => continue,
            Err(error) => {
                return Err(error).with_context(|| {
                    format!(
                        "parse Codex transcript line {} for commit evidence",
                        line_index + 1
                    )
                })
            }
        };
        if value.get("type").and_then(Value::as_str) != Some("response_item") {
            continue;
        }
        let Some(payload) = value.get("payload") else {
            continue;
        };
        match payload.get("type").and_then(Value::as_str) {
            Some("function_call") => {
                if let Some(call) = parse_commit_call(payload, fallback_cwd)? {
                    calls.insert(call.call_id.clone(), call);
                }
            }
            Some("function_call_output") => {
                let Some(call_id) = payload.get("call_id").and_then(Value::as_str) else {
                    continue;
                };
                let Some(call) = calls.remove(call_id) else {
                    continue;
                };
                let output = payload
                    .get("output")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if !codex_output_succeeded(output) {
                    continue;
                }
                let candidate = match commit_candidate_from_output(output) {
                    Ok(Some(candidate)) => candidate,
                    Ok(None) => continue,
                    Err(error) => {
                        crate::log::error(
                            "git-evidence",
                            &format!(
                                "skipped ambiguous successful commit output call_id={call_id}: {error}"
                            ),
                        );
                        continue;
                    }
                };
                let metadata = crate::git_util::resolve_commit_metadata(&call.cwd, &candidate)
                    .with_context(|| {
                        format!(
                            "resolve successful git commit candidate {candidate} call_id={call_id}"
                        )
                    })?;
                evidence
                    .entry(metadata.sha.clone())
                    .or_insert_with(|| GitCommitEvidence {
                        kind: GitEvidenceKind::ObservedCommit,
                        metadata,
                        locator: Some(format!("codex_call:{call_id}")),
                    });
            }
            _ => {}
        }
    }
    Ok(evidence.into_values().collect())
}

#[derive(Debug)]
struct CommitCall {
    call_id: String,
    cwd: String,
}

fn parse_commit_call(payload: &Value, fallback_cwd: &str) -> Result<Option<CommitCall>> {
    let Some(name) = payload.get("name").and_then(Value::as_str) else {
        return Ok(None);
    };
    if !matches!(name, "exec_command" | "shell" | "shell_command") {
        return Ok(None);
    }
    let call_id = payload
        .get("call_id")
        .and_then(Value::as_str)
        .context("Codex shell function call omitted call_id")?;
    let raw_arguments = payload
        .get("arguments")
        .and_then(Value::as_str)
        .context("Codex shell function call omitted string arguments")?;
    let arguments: Value =
        serde_json::from_str(raw_arguments).context("parse Codex shell function call arguments")?;
    let Some(command) = arguments
        .get("cmd")
        .or_else(|| arguments.get("command"))
        .and_then(Value::as_str)
    else {
        return Ok(None);
    };
    let workdir = arguments
        .get("workdir")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let base_cwd = match workdir {
        Some(workdir) => {
            let Some(resolved) = resolve_literal_workdir(Path::new(fallback_cwd), workdir) else {
                return Ok(None);
            };
            resolved
        }
        None => PathBuf::from(fallback_cwd),
    };
    let base_cwd = base_cwd.to_string_lossy();
    let Some(cwd) = commit_workdir(command, base_cwd.as_ref()) else {
        return Ok(None);
    };
    Ok(Some(CommitCall {
        call_id: call_id.to_string(),
        cwd: cwd.to_string_lossy().into_owned(),
    }))
}

fn hook_response_output(response: Option<&Value>) -> String {
    let Some(response) = response else {
        return String::new();
    };
    if let Some(text) = response.as_str() {
        return text.to_string();
    }
    ["stdout", "output", "content"]
        .into_iter()
        .filter_map(|key| response.get(key).and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("\n")
}

fn codex_output_succeeded(output: &str) -> bool {
    output
        .lines()
        .any(|line| line.trim() == "Process exited with code 0")
}

fn commit_candidate_from_output(output: &str) -> Result<Option<String>> {
    let pattern = Regex::new(r"(?m)^\[[^\]\r\n]*[ \t]([0-9a-fA-F]{7,64})\][^\r\n]*$")?;
    let candidates = pattern
        .captures_iter(output)
        .filter_map(|capture| capture.get(1))
        .map(|value| value.as_str().to_ascii_lowercase())
        .collect::<BTreeSet<_>>();
    match candidates.len() {
        1 => Ok(candidates.into_iter().next()),
        0 => Ok(None),
        count => bail!("successful git commit output contained {count} commit SHA candidates"),
    }
}

pub(crate) fn is_supported_commit_command(command: &str) -> bool {
    commit_workdir(command, ".").is_some()
}

fn commit_workdir(command: &str, base_cwd: &str) -> Option<PathBuf> {
    let parsed = parse_shell_command(command).ok()?;
    if parsed.separators.iter().any(|separator| separator != "&&") {
        return None;
    }
    let mut cwd = PathBuf::from(base_cwd);
    let mut commit_cwd = None;
    for segment in &parsed.segments {
        if commit_cwd.is_none() {
            if apply_literal_cd(segment, &mut cwd) {
                continue;
            }
            if is_supported_git_add(segment) {
                continue;
            }
            if let Some(resolved) = git_commit_workdir(segment, &cwd) {
                commit_cwd = Some(resolved);
                continue;
            }
            return None;
        }
        if !is_supported_post_commit_segment(segment) {
            return None;
        }
    }
    commit_cwd
}

#[derive(Debug, Default)]
struct ParsedShellCommand {
    segments: Vec<Vec<String>>,
    separators: Vec<String>,
}

fn parse_shell_command(command: &str) -> Result<ParsedShellCommand> {
    let mut parsed = ParsedShellCommand::default();
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut chars = command.chars().peekable();
    let mut quote = ShellQuote::None;
    let mut in_token = false;
    while let Some(ch) = chars.next() {
        match quote {
            ShellQuote::None => match ch {
                '\'' => {
                    quote = ShellQuote::Single;
                    in_token = true;
                }
                '"' => {
                    quote = ShellQuote::Double;
                    in_token = true;
                }
                '\\' => {
                    in_token = true;
                    if let Some(next) = chars.next() {
                        current.push(next);
                    }
                }
                ';' | '|' | '&' | '\n' => {
                    push_token(&mut tokens, &mut current, &mut in_token);
                    if tokens.is_empty() {
                        bail!("empty shell segment around commit command separator");
                    }
                    parsed.segments.push(std::mem::take(&mut tokens));
                    let mut separator = ch.to_string();
                    if matches!(ch, '|' | '&') && chars.peek() == Some(&ch) {
                        separator.push(chars.next().unwrap_or(ch));
                    }
                    parsed.separators.push(separator);
                }
                value if value.is_whitespace() => {
                    push_token(&mut tokens, &mut current, &mut in_token)
                }
                _ => {
                    current.push(ch);
                    in_token = true;
                }
            },
            ShellQuote::Single => {
                if ch == '\'' {
                    quote = ShellQuote::None;
                } else {
                    current.push(ch);
                }
            }
            ShellQuote::Double => match ch {
                '"' => quote = ShellQuote::None,
                '\\' => {
                    if let Some(next) = chars.next() {
                        current.push(next);
                    }
                }
                _ => current.push(ch),
            },
        }
    }
    if quote != ShellQuote::None {
        bail!("unclosed shell quote in commit command");
    }
    push_token(&mut tokens, &mut current, &mut in_token);
    if !tokens.is_empty() {
        parsed.segments.push(tokens);
    }
    if parsed.segments.is_empty() || parsed.separators.len() + 1 != parsed.segments.len() {
        bail!("incomplete shell command around commit evidence");
    }
    Ok(parsed)
}

fn push_token(tokens: &mut Vec<String>, current: &mut String, in_token: &mut bool) {
    if *in_token {
        tokens.push(std::mem::take(current));
        *in_token = false;
    }
}

fn apply_literal_cd(tokens: &[String], cwd: &mut PathBuf) -> bool {
    let Some(index) = tokens.iter().position(|token| !is_env_assignment(token)) else {
        return false;
    };
    if tokens.get(index).map(String::as_str) != Some("cd") {
        return false;
    }
    let target = match &tokens[index + 1..] {
        [target] => target.as_str(),
        [flag, target] if flag == "--" => target.as_str(),
        _ => return false,
    };
    let Some(next) = resolve_literal_workdir(cwd, target) else {
        return false;
    };
    *cwd = next;
    true
}

fn is_supported_git_add(tokens: &[String]) -> bool {
    matches!(git_subcommand(tokens, Path::new(".")), Some(("add", _)))
}

fn is_supported_post_commit_segment(tokens: &[String]) -> bool {
    matches!(tokens, [git, status, short] if git == "git" && status == "status" && short == "--short")
}

fn git_commit_workdir(tokens: &[String], base_cwd: &Path) -> Option<PathBuf> {
    let (subcommand, cwd) = git_subcommand(tokens, base_cwd)?;
    (subcommand == "commit").then_some(cwd)
}

fn git_subcommand<'a>(tokens: &'a [String], base_cwd: &Path) -> Option<(&'a str, PathBuf)> {
    let mut index = tokens.iter().position(|token| !is_env_assignment(token))?;
    if tokens.get(index).map(String::as_str) != Some("git") {
        return None;
    }
    index += 1;
    let mut cwd = base_cwd.to_path_buf();
    while let Some(token) = tokens.get(index) {
        match token.as_str() {
            "-C" => {
                let target = tokens.get(index + 1)?;
                cwd = resolve_literal_workdir(&cwd, target)?;
                index += 2;
            }
            "-c" => index += 2,
            "-p"
            | "--paginate"
            | "-P"
            | "--no-pager"
            | "--bare"
            | "--no-replace-objects"
            | "--literal-pathspecs"
            | "--glob-pathspecs"
            | "--noglob-pathspecs"
            | "--icase-pathspecs"
            | "--no-optional-locks" => index += 1,
            value if value.starts_with("-C") && value.len() > 2 => {
                cwd = resolve_literal_workdir(&cwd, &value[2..])?;
                index += 1;
            }
            value if value.starts_with("-c") && value.len() > 2 => index += 1,
            subcommand => return Some((subcommand, cwd)),
        }
    }
    None
}

fn resolve_literal_workdir(base: &Path, target: &str) -> Option<PathBuf> {
    if target.is_empty() || target.starts_with('~') || target.contains('$') || target.contains('`')
    {
        return None;
    }
    let target = Path::new(target);
    Some(if target.is_absolute() {
        target.to_path_buf()
    } else {
        base.join(target)
    })
}

fn is_env_assignment(token: &str) -> bool {
    let Some((name, _)) = token.split_once('=') else {
        return false;
    };
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == '_')
        && chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShellQuote {
    None,
    Single,
    Double,
}

#[cfg(test)]
#[path = "git_evidence/tests.rs"]
mod tests;
