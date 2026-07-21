use std::path::Path;

use serde::{Deserialize, Serialize};

mod bash_ast;

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
    Ok(segments.iter().any(|tokens| {
        git_subcommand_args(tokens, "push").is_some_and(git_push_args_force)
            || git_alias_forces_push(tokens)
    }))
}

fn git_subcommand_args<'a>(tokens: &'a [String], expected: &str) -> Option<&'a [String]> {
    let index = git_subcommand_index(tokens)?;
    (tokens.get(index)? == expected).then_some(&tokens[index + 1..])
}

fn git_subcommand_index(tokens: &[String]) -> Option<usize> {
    let command_index = bash_ast::unwrap::effective_command_index(tokens)?;
    if !is_git_executable(bash_ast::unwrap::semantic_token(tokens.get(command_index)?)) {
        return None;
    }
    let mut index = command_index;
    index += 1;

    while let Some(token) = tokens.get(index) {
        match token.as_str() {
            "-C" | "-c" | "--config-env" | "--exec-path" | "--git-dir" | "--work-tree"
            | "--namespace" | "--super-prefix" => {
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
                    || value.starts_with("--config-env=")
                    || value.starts_with("--git-dir=")
                    || value.starts_with("--work-tree=")
                    || value.starts_with("--namespace=")
                    || value.starts_with("--super-prefix=") =>
            {
                index += 1;
            }
            _ => return Some(index),
        }
    }

    None
}

fn git_alias_forces_push(tokens: &[String]) -> bool {
    let Some(subcommand_index) = git_subcommand_index(tokens) else {
        return false;
    };
    let Some(alias_name) = tokens.get(subcommand_index) else {
        return false;
    };
    let Some(payload) = git_config_alias_payload(tokens, subcommand_index, alias_name) else {
        return false;
    };
    let shell_alias = payload.starts_with('!');
    let payload = payload.strip_prefix('!').unwrap_or(payload);
    if shell_alias {
        let Ok(segments) = shell_command_segments(payload) else {
            return false;
        };
        return segments.into_iter().any(|mut segment| {
            segment.extend_from_slice(&tokens[subcommand_index + 1..]);
            git_subcommand_args(&segment, "push").is_some_and(git_push_args_force)
        });
    }

    let Some(mut alias_args) = split_git_alias(payload) else {
        return false;
    };
    alias_args.extend_from_slice(&tokens[subcommand_index + 1..]);
    alias_args
        .strip_prefix(&["push".to_string()])
        .is_some_and(git_push_args_force)
}

fn split_git_alias(payload: &str) -> Option<Vec<String>> {
    let mut arguments = Vec::new();
    let mut argument = String::new();
    let mut quoted = None;
    let mut escaped = false;
    let mut started = false;

    for ch in payload.chars() {
        if escaped {
            argument.push(ch);
            escaped = false;
            started = true;
            continue;
        }
        if ch == '\\' && quoted != Some('\'') {
            escaped = true;
            started = true;
            continue;
        }
        if matches!(ch, '\'' | '\"') {
            if quoted == Some(ch) {
                quoted = None;
            } else if quoted.is_none() {
                quoted = Some(ch);
            } else {
                argument.push(ch);
            }
            started = true;
            continue;
        }
        if quoted.is_none() && ch.is_ascii_whitespace() {
            if started {
                arguments.push(std::mem::take(&mut argument));
                started = false;
            }
            continue;
        }
        argument.push(ch);
        started = true;
    }

    if escaped || quoted.is_some() {
        return None;
    }
    if started {
        arguments.push(argument);
    }
    Some(arguments)
}

fn git_config_alias_payload<'a>(
    tokens: &'a [String],
    subcommand_index: usize,
    alias_name: &str,
) -> Option<&'a str> {
    let command_index = bash_ast::unwrap::effective_command_index(tokens)?;
    let mut index = command_index + 1;
    let mut payload = None;
    while index < subcommand_index {
        let token = tokens.get(index)?;
        let assignment = if token == "-c" {
            index += 2;
            tokens.get(index - 1)?.as_str()
        } else if let Some(assignment) = token.strip_prefix("-c") {
            index += 1;
            assignment
        } else {
            index += 1;
            continue;
        };
        if let Some((key, value)) = assignment.split_once('=') {
            if let Some((section, name)) = key.split_once('.') {
                if section.eq_ignore_ascii_case("alias") && name.eq_ignore_ascii_case(alias_name) {
                    payload = Some(value);
                }
            }
        }
    }
    payload
}

fn is_git_executable(command: &str) -> bool {
    let basename = command.rsplit(['/', '\\']).next().unwrap_or(command);
    basename == "git" || basename.eq_ignore_ascii_case("git.exe")
}

fn git_push_args_force(args: &[String]) -> bool {
    let mut index = 0;
    let mut repository_supplied = false;
    let mut options_terminated = false;
    let mut force_enabled = false;
    let mut mirror_enabled = false;
    let mut delete_enabled = false;
    while let Some(arg) = args.get(index) {
        if !options_terminated && arg == "--" {
            options_terminated = true;
            index += 1;
            continue;
        }
        if !options_terminated && arg == "--force" {
            force_enabled = true;
            index += 1;
            continue;
        }
        if !options_terminated && arg == "--no-force" {
            force_enabled = false;
            index += 1;
            continue;
        }
        if !options_terminated {
            if let Some(enabled) = mirror_option_state(arg) {
                mirror_enabled = enabled;
                index += 1;
                continue;
            }
            if let Some(enabled) = delete_option_state(arg) {
                delete_enabled = enabled;
                index += 1;
                continue;
            }
        }
        if !options_terminated && (arg == "--repo" || arg.starts_with("--repo=")) {
            repository_supplied = arg.starts_with("--repo=") || args.get(index + 1).is_some();
            index += if arg == "--repo" { 2 } else { 1 };
            continue;
        }
        if !options_terminated {
            if git_push_short_option_enables_delete(arg) {
                delete_enabled = true;
            }
            match git_push_short_option_effect(arg) {
                PushShortOptionEffect::Forces => {
                    force_enabled = true;
                    index += 1;
                    continue;
                }
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
        if repository_supplied && !delete_enabled && is_force_push_refspec(arg) {
            return true;
        }
        repository_supplied = true;
        index += 1;
    }
    force_enabled || mirror_enabled
}

fn git_push_short_option_enables_delete(arg: &str) -> bool {
    let Some(cluster) = arg
        .strip_prefix('-')
        .filter(|value| !value.starts_with('-'))
    else {
        return false;
    };
    for option in cluster.chars() {
        match option {
            'd' => return true,
            'o' => return false,
            _ => {}
        }
    }
    false
}

fn delete_option_state(arg: &str) -> Option<bool> {
    if let Some(prefix) = arg.strip_prefix("--no-") {
        return (!prefix.is_empty() && "delete".starts_with(prefix)).then_some(false);
    }
    let prefix = arg.strip_prefix("--")?;
    (!prefix.is_empty() && "delete".starts_with(prefix)).then_some(true)
}

pub(super) fn git_push_arg_changes_force_state(arg: &str) -> bool {
    matches!(arg, "--force" | "--no-force")
        || mirror_option_state(arg).is_some()
        || git_push_short_option_effect(arg) == PushShortOptionEffect::Forces
        || is_force_push_refspec(arg)
}

fn mirror_option_state(arg: &str) -> Option<bool> {
    if let Some(prefix) = arg.strip_prefix("--no-") {
        return (!prefix.is_empty() && "mirror".starts_with(prefix)).then_some(false);
    }
    let prefix = arg.strip_prefix("--")?;
    (!prefix.is_empty() && "mirror".starts_with(prefix)).then_some(true)
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
    bash_ast::command_segments(command)
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
