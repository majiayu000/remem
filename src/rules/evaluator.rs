use std::collections::VecDeque;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::rules::artifact::{
    CompiledRule, CompiledRulesArtifact, RuleAction, RulePredicate, LEGACY_ARTIFACT_VERSION,
};
use crate::rules::store::{load_artifact_fail_open, ArtifactLoad, ArtifactLoadErrorKind};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvaluationInput {
    pub command: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvaluationOutcome {
    pub verdict: EvaluationVerdict,
    pub matches: Vec<RuleMatch>,
    pub diagnostics: Vec<EvaluationDiagnostic>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvaluationVerdict {
    Allow,
    Warn,
    Block,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleMatch {
    pub rule_id: String,
    pub source_memory_id: i64,
    pub action: RuleAction,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvaluationDiagnostic {
    pub status: EvaluationDiagnosticStatus,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CodedEvaluationOutcome {
    pub outcome: EvaluationOutcome,
    pub diagnostic_codes: Vec<EvaluationDiagnosticCode>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum EvaluationDiagnosticCode {
    ArtifactMissing,
    ArtifactRead,
    ArtifactParse,
    ArtifactValidate,
    RuleEvaluation,
    HookInputRead,
    Config,
    HookInput,
    OutputSerialize,
}

impl EvaluationDiagnosticCode {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::ArtifactMissing => "artifact_missing",
            Self::ArtifactRead => "artifact_read",
            Self::ArtifactParse => "artifact_parse",
            Self::ArtifactValidate => "artifact_validate",
            Self::RuleEvaluation => "rule_evaluation",
            Self::HookInputRead => "hook_input_read",
            Self::Config => "config",
            Self::HookInput => "hook_input",
            Self::OutputSerialize => "output_serialize",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvaluationDiagnosticStatus {
    Error,
}

pub fn evaluate_artifact(
    artifact: &CompiledRulesArtifact,
    input: &EvaluationInput,
) -> EvaluationOutcome {
    evaluate_artifact_with_codes(artifact, input).outcome
}

fn evaluate_artifact_with_codes(
    artifact: &CompiledRulesArtifact,
    input: &EvaluationInput,
) -> CodedEvaluationOutcome {
    let mut matches = Vec::new();
    let mut diagnostics = Vec::new();
    let mut diagnostic_codes = Vec::new();

    for rule in &artifact.rules {
        if rule.override_state.disabled {
            continue;
        }
        match rule_matches(artifact.version, rule, input) {
            Ok(true) => matches.push(RuleMatch {
                rule_id: rule.rule_id.clone(),
                source_memory_id: rule.source_memory_id,
                action: rule.effective_action(),
                message: rule.predicate.message().to_string(),
            }),
            Ok(false) => {}
            Err(message) => {
                diagnostics.push(EvaluationDiagnostic {
                    status: EvaluationDiagnosticStatus::Error,
                    message,
                });
                diagnostic_codes.push(EvaluationDiagnosticCode::RuleEvaluation);
            }
        }
    }

    if !diagnostics.is_empty() {
        return CodedEvaluationOutcome {
            outcome: EvaluationOutcome {
                verdict: EvaluationVerdict::Allow,
                matches: Vec::new(),
                diagnostics,
            },
            diagnostic_codes,
        };
    }

    CodedEvaluationOutcome {
        outcome: EvaluationOutcome {
            verdict: verdict_for_matches(&matches),
            matches,
            diagnostics,
        },
        diagnostic_codes,
    }
}

pub fn evaluate_artifact_file(
    path: impl AsRef<Path>,
    input: &EvaluationInput,
) -> EvaluationOutcome {
    evaluate_artifact_file_with_codes(path, input).outcome
}

pub(crate) fn evaluate_artifact_file_with_codes(
    path: impl AsRef<Path>,
    input: &EvaluationInput,
) -> CodedEvaluationOutcome {
    match load_artifact_fail_open(path) {
        ArtifactLoad::Loaded(artifact) => evaluate_artifact_with_codes(&artifact, input),
        ArtifactLoad::FailOpen { kind, message } => CodedEvaluationOutcome {
            outcome: EvaluationOutcome {
                verdict: EvaluationVerdict::Allow,
                matches: Vec::new(),
                diagnostics: vec![EvaluationDiagnostic {
                    status: EvaluationDiagnosticStatus::Error,
                    message,
                }],
            },
            diagnostic_codes: vec![diagnostic_code_for_artifact_error(kind)],
        },
    }
}

fn diagnostic_code_for_artifact_error(kind: ArtifactLoadErrorKind) -> EvaluationDiagnosticCode {
    match kind {
        ArtifactLoadErrorKind::Missing => EvaluationDiagnosticCode::ArtifactMissing,
        ArtifactLoadErrorKind::Read => EvaluationDiagnosticCode::ArtifactRead,
        ArtifactLoadErrorKind::Parse => EvaluationDiagnosticCode::ArtifactParse,
        ArtifactLoadErrorKind::Validate => EvaluationDiagnosticCode::ArtifactValidate,
    }
}

fn rule_matches(
    artifact_version: u32,
    rule: &CompiledRule,
    input: &EvaluationInput,
) -> Result<bool, String> {
    match &rule.predicate {
        RulePredicate::CommandRegex { pattern, .. } => {
            if artifact_version == LEGACY_ARTIFACT_VERSION {
                regex::Regex::new(pattern)
                    .map(|regex| regex.is_match(&input.command))
                    .map_err(|err| format!("rule {} has invalid regex: {err}", rule.rule_id))
            } else {
                regex_lite::Regex::new(pattern)
                    .map(|regex| regex.is_match(&input.command))
                    .map_err(|err| format!("rule {} has invalid regex: {err}", rule.rule_id))
            }
        }
        RulePredicate::CommitTrailerForbidden { trailer, .. } => {
            command_adds_forbidden_commit_trailer(&input.command, trailer)
                .map_err(|err| format!("rule {} could not parse command: {err}", rule.rule_id))
        }
        RulePredicate::GitPushForceForbidden { .. } => command_forces_git_push(&input.command)
            .map_err(|err| format!("rule {} could not parse command: {err}", rule.rule_id)),
    }
}

fn command_adds_forbidden_commit_trailer(command: &str, trailer: &str) -> Result<bool, String> {
    let segments = shell_command_segments(command)?;
    Ok(segments
        .iter()
        .any(|tokens| git_commit_segment_adds_trailer(tokens, trailer)))
}

fn git_commit_segment_adds_trailer(tokens: &[String], trailer: &str) -> bool {
    git_subcommand_args(tokens, "commit").is_some_and(|args| commit_args_add_trailer(args, trailer))
}

fn command_forces_git_push(command: &str) -> Result<bool, String> {
    let segments = shell_command_segments(command)?;
    Ok(segments
        .iter()
        .any(|tokens| git_subcommand_args(tokens, "push").is_some_and(git_push_args_force)))
}

fn git_subcommand_args<'a>(tokens: &'a [String], expected: &str) -> Option<&'a [String]> {
    let command_index = tokens.iter().position(|token| !is_env_assignment(token))?;
    if tokens[command_index] != "git" {
        return None;
    }
    let mut index = command_index;
    index += 1;

    while let Some(token) = tokens.get(index) {
        match token.as_str() {
            value if value == expected => return Some(&tokens[index + 1..]),
            "-C" | "-c" | "--exec-path" | "--git-dir" | "--work-tree" | "--namespace"
            | "--super-prefix" => {
                index += 2;
            }
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
            | "--no-optional-locks" => {
                index += 1;
            }
            value
                if value.starts_with("-C")
                    || value.starts_with("-c")
                    || value.starts_with("--exec-path=")
                    || value.starts_with("--git-dir=")
                    || value.starts_with("--work-tree=")
                    || value.starts_with("--namespace=")
                    || value.starts_with("--super-prefix=") =>
            {
                index += 1;
            }
            _ => return None,
        }
    }

    None
}

fn git_push_args_force(args: &[String]) -> bool {
    let mut index = 0;
    let mut repository_supplied = false;
    let mut options_terminated = false;
    while let Some(arg) = args.get(index) {
        if !options_terminated && arg == "--" {
            options_terminated = true;
            index += 1;
            continue;
        }
        if !options_terminated && matches!(arg.as_str(), "--force" | "-f") {
            return true;
        }
        if !options_terminated && (arg == "--repo" || arg.starts_with("--repo=")) {
            repository_supplied = arg.starts_with("--repo=") || args.get(index + 1).is_some();
            index += if arg == "--repo" { 2 } else { 1 };
            continue;
        }
        if !options_terminated {
            match git_push_short_option_effect(arg) {
                PushShortOptionEffect::Forces => return true,
                PushShortOptionEffect::ConsumesNext => {
                    index += 2;
                    continue;
                }
                PushShortOptionEffect::Other => {}
            }
            if arg.starts_with('-') {
                index += if git_push_long_option_consumes_next(arg) {
                    2
                } else {
                    1
                };
                continue;
            }
        }
        if repository_supplied && is_force_push_refspec(arg) {
            return true;
        }
        repository_supplied = true;
        index += 1;
    }
    false
}

fn is_force_push_refspec(arg: &str) -> bool {
    let Some(refspec) = arg.strip_prefix('+') else {
        return false;
    };
    let source = refspec
        .split_once(':')
        .map_or(refspec, |(source, _)| source);
    !source.is_empty() && !source.starts_with('+')
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PushShortOptionEffect {
    Forces,
    ConsumesNext,
    Other,
}

fn git_push_short_option_effect(arg: &str) -> PushShortOptionEffect {
    let Some(cluster) = arg
        .strip_prefix('-')
        .filter(|value| !value.starts_with('-'))
    else {
        return PushShortOptionEffect::Other;
    };
    let chars = cluster.chars().collect::<Vec<_>>();
    for (index, option) in chars.iter().enumerate() {
        match option {
            'f' => return PushShortOptionEffect::Forces,
            'o' if index + 1 == chars.len() => return PushShortOptionEffect::ConsumesNext,
            'o' => return PushShortOptionEffect::Other,
            _ => {}
        }
    }
    PushShortOptionEffect::Other
}

fn git_push_long_option_consumes_next(arg: &str) -> bool {
    matches!(
        arg,
        "--push-option" | "--receive-pack" | "--exec" | "--repo" | "--recurse-submodules"
    )
}

fn commit_args_add_trailer(args: &[String], trailer: &str) -> bool {
    let mut index = 0;
    while let Some(arg) = args.get(index) {
        if arg == "--" {
            return false;
        }
        if commit_option_consumes_next(arg) {
            index += 2;
            continue;
        }
        if arg == "--trailer" {
            if args
                .get(index + 1)
                .is_some_and(|value| trailer_arg_matches(value, trailer))
            {
                return true;
            }
            index += 2;
            continue;
        }
        if let Some(value) = arg.strip_prefix("--trailer=") {
            if trailer_arg_matches(value, trailer) {
                return true;
            }
        }
        index += 1;
    }

    false
}

fn commit_option_consumes_next(arg: &str) -> bool {
    if matches!(
        arg,
        "-m" | "-F"
            | "-C"
            | "-c"
            | "--message"
            | "--file"
            | "--reuse-message"
            | "--reedit-message"
            | "--author"
            | "--date"
            | "--cleanup"
            | "--template"
            | "--fixup"
            | "--squash"
            | "--pathspec-from-file"
    ) {
        return true;
    }
    if arg.starts_with("--") {
        return false;
    }

    arg.len() > 2
        && arg
            .chars()
            .last()
            .is_some_and(|ch| matches!(ch, 'm' | 'F' | 'C' | 'c'))
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

fn trailer_arg_matches(value: &str, trailer: &str) -> bool {
    let trimmed = value.trim_start();
    if trimmed == trailer {
        return true;
    }
    let Some(rest) = trimmed.strip_prefix(trailer) else {
        return false;
    };
    rest.chars()
        .next()
        .is_some_and(|ch| matches!(ch, '=' | ':'))
}

fn shell_command_segments(command: &str) -> Result<Vec<Vec<String>>, String> {
    let command = shell_without_heredoc_bodies(command)?;
    let mut segments = Vec::new();
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
                    if let Some(next) = chars.next() {
                        if next != '\n' {
                            current.push(next);
                            in_token = true;
                        }
                    }
                }
                ';' | '|' | '&' | '\n' | '(' | ')' => {
                    push_token(&mut tokens, &mut current, &mut in_token);
                    push_segment(&mut segments, &mut tokens);
                    if matches!(ch, ';' | '|' | '&')
                        && matches!(chars.peek(), Some(next) if *next == ch)
                    {
                        chars.next();
                    }
                }
                '{' | '}'
                    if tokens.is_empty()
                        && chars.peek().is_none_or(|next| {
                            next.is_whitespace() || matches!(next, ';' | '|' | '&' | '(' | ')')
                        }) => {}
                ch if ch.is_whitespace() => {
                    push_token(&mut tokens, &mut current, &mut in_token);
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
                        if next != '\n' {
                            current.push(next);
                        }
                    }
                }
                _ => current.push(ch),
            },
        }
    }

    if quote != ShellQuote::None {
        return Err("unclosed shell quote".to_string());
    }
    push_token(&mut tokens, &mut current, &mut in_token);
    push_segment(&mut segments, &mut tokens);
    Ok(segments)
}

fn shell_without_heredoc_bodies(command: &str) -> Result<String, String> {
    let mut executable = String::with_capacity(command.len());
    let mut pending = VecDeque::new();
    for line in command.split_inclusive('\n') {
        if let Some((delimiter, strip_tabs)) = pending.front() {
            let candidate = line.strip_suffix('\n').unwrap_or(line);
            let candidate = candidate.strip_suffix('\r').unwrap_or(candidate);
            let candidate = if *strip_tabs {
                candidate.trim_start_matches('\t')
            } else {
                candidate
            };
            if candidate == delimiter {
                pending.pop_front();
            }
            continue;
        }
        executable.push_str(line);
        pending.extend(heredoc_declarations(line)?);
    }
    Ok(executable)
}

fn heredoc_declarations(line: &str) -> Result<Vec<(String, bool)>, String> {
    let mut declarations = Vec::new();
    let mut chars = line.chars().peekable();
    let mut quote = ShellQuote::None;
    let mut at_word_start = true;
    while let Some(ch) = chars.next() {
        match quote {
            ShellQuote::Single => {
                if ch == '\'' {
                    quote = ShellQuote::None;
                }
            }
            ShellQuote::Double => match ch {
                '"' => quote = ShellQuote::None,
                '\\' => {
                    chars.next();
                }
                _ => {}
            },
            ShellQuote::None => match ch {
                '#' if at_word_start => break,
                '\'' => {
                    quote = ShellQuote::Single;
                    at_word_start = false;
                }
                '"' => {
                    quote = ShellQuote::Double;
                    at_word_start = false;
                }
                '\\' => {
                    chars.next();
                    at_word_start = false;
                }
                '<' if chars.peek() == Some(&'<') => {
                    chars.next();
                    if chars.peek() == Some(&'<') {
                        chars.next();
                        at_word_start = true;
                        continue;
                    }
                    let strip_tabs = chars.next_if_eq(&'-').is_some();
                    while chars.next_if(|next| matches!(next, ' ' | '\t')).is_some() {}
                    declarations.push((heredoc_delimiter(&mut chars)?, strip_tabs));
                    at_word_start = false;
                }
                ch if ch.is_whitespace() || matches!(ch, ';' | '|' | '&' | '(' | ')') => {
                    at_word_start = true;
                }
                _ => at_word_start = false,
            },
        }
    }
    Ok(declarations)
}

fn heredoc_delimiter(
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
) -> Result<String, String> {
    let mut delimiter = String::new();
    let mut quote = ShellQuote::None;
    let mut present = false;
    while let Some(&ch) = chars.peek() {
        if quote == ShellQuote::None
            && (ch.is_whitespace() || matches!(ch, ';' | '|' | '&' | '(' | ')' | '<' | '>'))
        {
            break;
        }
        let value = chars.next().ok_or("missing peeked heredoc delimiter")?;
        match quote {
            ShellQuote::None => match value {
                '\'' => {
                    quote = ShellQuote::Single;
                    present = true;
                }
                '"' => {
                    quote = ShellQuote::Double;
                    present = true;
                }
                '\\' => {
                    let escaped = chars
                        .next()
                        .ok_or("missing heredoc delimiter after escape")?;
                    delimiter.push(escaped);
                    present = true;
                }
                value => {
                    delimiter.push(value);
                    present = true;
                }
            },
            ShellQuote::Single => match value {
                '\'' => quote = ShellQuote::None,
                value => delimiter.push(value),
            },
            ShellQuote::Double => match value {
                '"' => quote = ShellQuote::None,
                '\\' => delimiter.push(
                    chars
                        .next()
                        .ok_or("missing quoted heredoc delimiter after escape")?,
                ),
                value => delimiter.push(value),
            },
        }
    }
    if !present || quote != ShellQuote::None {
        return Err("invalid heredoc delimiter".to_string());
    }
    Ok(delimiter)
}

fn push_token(tokens: &mut Vec<String>, current: &mut String, in_token: &mut bool) {
    if *in_token {
        tokens.push(std::mem::take(current));
        *in_token = false;
    }
}

fn push_segment(segments: &mut Vec<Vec<String>>, tokens: &mut Vec<String>) {
    if !tokens.is_empty() {
        segments.push(std::mem::take(tokens));
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ShellQuote {
    None,
    Single,
    Double,
}

fn verdict_for_matches(matches: &[RuleMatch]) -> EvaluationVerdict {
    if matches
        .iter()
        .any(|rule_match| rule_match.action == RuleAction::Block)
    {
        EvaluationVerdict::Block
    } else if matches.is_empty() {
        EvaluationVerdict::Allow
    } else {
        EvaluationVerdict::Warn
    }
}

#[cfg(test)]
mod tests;
