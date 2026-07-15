use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::rules::artifact::{CompiledRule, CompiledRulesArtifact, RuleAction, RulePredicate};
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
    pub code: EvaluationDiagnosticCode,
    pub status: EvaluationDiagnosticStatus,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvaluationDiagnosticCode {
    ArtifactMissing,
    ArtifactRead,
    ArtifactParse,
    ArtifactValidate,
    RuleEvaluation,
}

impl EvaluationDiagnosticCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ArtifactMissing => "artifact_missing",
            Self::ArtifactRead => "artifact_read",
            Self::ArtifactParse => "artifact_parse",
            Self::ArtifactValidate => "artifact_validate",
            Self::RuleEvaluation => "rule_evaluation",
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
    let mut matches = Vec::new();
    let mut diagnostics = Vec::new();

    for rule in &artifact.rules {
        if rule.override_state.disabled {
            continue;
        }
        match rule_matches(rule, input) {
            Ok(true) => matches.push(RuleMatch {
                rule_id: rule.rule_id.clone(),
                source_memory_id: rule.source_memory_id,
                action: rule.effective_action(),
                message: rule.predicate.message().to_string(),
            }),
            Ok(false) => {}
            Err(message) => diagnostics.push(EvaluationDiagnostic {
                code: EvaluationDiagnosticCode::RuleEvaluation,
                status: EvaluationDiagnosticStatus::Error,
                message,
            }),
        }
    }

    if !diagnostics.is_empty() {
        return EvaluationOutcome {
            verdict: EvaluationVerdict::Allow,
            matches: Vec::new(),
            diagnostics,
        };
    }

    EvaluationOutcome {
        verdict: verdict_for_matches(&matches),
        matches,
        diagnostics,
    }
}

pub fn evaluate_artifact_file(
    path: impl AsRef<Path>,
    input: &EvaluationInput,
) -> EvaluationOutcome {
    match load_artifact_fail_open(path) {
        ArtifactLoad::Loaded(artifact) => evaluate_artifact(&artifact, input),
        ArtifactLoad::FailOpen { kind, message } => EvaluationOutcome {
            verdict: EvaluationVerdict::Allow,
            matches: Vec::new(),
            diagnostics: vec![EvaluationDiagnostic {
                code: diagnostic_code_for_artifact_error(kind),
                status: EvaluationDiagnosticStatus::Error,
                message,
            }],
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

fn rule_matches(rule: &CompiledRule, input: &EvaluationInput) -> Result<bool, String> {
    match &rule.predicate {
        RulePredicate::CommandRegex { pattern, .. } => regex::Regex::new(pattern)
            .map(|regex| regex.is_match(&input.command))
            .map_err(|err| format!("rule {} has invalid regex: {err}", rule.rule_id)),
        RulePredicate::CommitTrailerForbidden { trailer, .. } => {
            command_adds_forbidden_commit_trailer(&input.command, trailer)
                .map_err(|err| format!("rule {} could not parse command: {err}", rule.rule_id))
        }
    }
}

fn command_adds_forbidden_commit_trailer(command: &str, trailer: &str) -> Result<bool, String> {
    let segments = shell_command_segments(command)?;
    Ok(segments
        .iter()
        .any(|tokens| git_commit_segment_adds_trailer(tokens, trailer)))
}

fn git_commit_segment_adds_trailer(tokens: &[String], trailer: &str) -> bool {
    let Some(command_index) = tokens.iter().position(|token| !is_env_assignment(token)) else {
        return false;
    };
    if tokens[command_index] != "git" {
        return false;
    }
    let mut index = command_index;
    index += 1;

    while let Some(token) = tokens.get(index) {
        match token.as_str() {
            "commit" => return commit_args_add_trailer(&tokens[index + 1..], trailer),
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
            _ => return false,
        }
    }

    false
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
                    in_token = true;
                    if let Some(next) = chars.next() {
                        current.push(next);
                    }
                }
                ';' | '|' | '&' => {
                    push_token(&mut tokens, &mut current, &mut in_token);
                    push_segment(&mut segments, &mut tokens);
                    if matches!(chars.peek(), Some(next) if *next == ch) {
                        chars.next();
                    }
                }
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
                        current.push(next);
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
mod tests {
    use super::*;
    use crate::rules::artifact::{CompiledRule, RuleOverrideState};
    use crate::rules::test_support::package_manager_rule;

    fn forbidden_trailer_rule() -> CompiledRule {
        CompiledRule {
            rule_id: "pref-456-1".to_string(),
            source_memory_id: 456,
            reinforcement_count: 4,
            action: RuleAction::Block,
            override_state: RuleOverrideState {
                disabled: false,
                action_override: None,
            },
            predicate: RulePredicate::CommitTrailerForbidden {
                trailer: "AI-generated-by".to_string(),
                message: "Do not add AI-generated commit trailers".to_string(),
            },
        }
    }

    #[test]
    fn evaluator_is_deterministic_for_same_input_and_artifact() {
        let artifact = CompiledRulesArtifact::new(
            99,
            vec![
                package_manager_rule(RuleAction::Warn),
                forbidden_trailer_rule(),
            ],
        );
        let input = EvaluationInput {
            command: "npm install && git commit -m init --trailer AI-generated-by=bot".to_string(),
        };

        let first = evaluate_artifact(&artifact, &input);
        let second = evaluate_artifact(&artifact, &input);

        assert_eq!(first, second);
        assert_eq!(first.verdict, EvaluationVerdict::Block);
        assert_eq!(first.matches.len(), 2);
        assert!(first.diagnostics.is_empty());
    }

    #[test]
    fn evaluator_skips_disabled_rules_and_warns_by_default() {
        let mut disabled = package_manager_rule(RuleAction::Warn);
        disabled.override_state.disabled = true;
        let artifact =
            CompiledRulesArtifact::new(99, vec![disabled, package_manager_rule(RuleAction::Warn)]);
        let input = EvaluationInput {
            command: "npm add left-pad".to_string(),
        };

        let outcome = evaluate_artifact(&artifact, &input);

        assert_eq!(outcome.verdict, EvaluationVerdict::Warn);
        assert_eq!(outcome.matches.len(), 1);
        assert_eq!(outcome.matches[0].rule_id, "pref-123-1");
    }

    #[test]
    fn invalid_regex_fails_open_for_that_rule() {
        let artifact = CompiledRulesArtifact::new(
            99,
            vec![CompiledRule {
                predicate: RulePredicate::CommandRegex {
                    pattern: "(".to_string(),
                    message: "broken".to_string(),
                },
                ..package_manager_rule(RuleAction::Block)
            }],
        );
        let input = EvaluationInput {
            command: "npm install".to_string(),
        };

        let outcome = evaluate_artifact(&artifact, &input);

        assert_eq!(outcome.verdict, EvaluationVerdict::Allow);
        assert!(outcome.matches.is_empty());
        assert_eq!(outcome.diagnostics.len(), 1);
        assert!(outcome.diagnostics[0].message.contains("invalid regex"));
    }

    #[test]
    fn commit_trailer_rule_handles_git_global_options() {
        let artifact = CompiledRulesArtifact::new(99, vec![forbidden_trailer_rule()]);
        let input = EvaluationInput {
            command: "git -C /repo -c user.email=x commit -m init --trailer AI-generated-by=bot"
                .to_string(),
        };

        let outcome = evaluate_artifact(&artifact, &input);

        assert_eq!(outcome.verdict, EvaluationVerdict::Block);
        assert_eq!(outcome.matches.len(), 1);
        assert!(outcome.diagnostics.is_empty());
    }

    #[test]
    fn commit_trailer_rule_ignores_message_text_mentions() {
        let artifact = CompiledRulesArtifact::new(99, vec![forbidden_trailer_rule()]);
        let input = EvaluationInput {
            command: "git commit -m 'remove AI-generated-by support'".to_string(),
        };

        let outcome = evaluate_artifact(&artifact, &input);

        assert_eq!(outcome.verdict, EvaluationVerdict::Allow);
        assert!(outcome.matches.is_empty());
        assert!(outcome.diagnostics.is_empty());
    }

    #[test]
    fn commit_trailer_rule_requires_git_as_segment_command() {
        let artifact = CompiledRulesArtifact::new(99, vec![forbidden_trailer_rule()]);
        let input = EvaluationInput {
            command: "echo git commit --trailer AI-generated-by=bot".to_string(),
        };

        let outcome = evaluate_artifact(&artifact, &input);

        assert_eq!(outcome.verdict, EvaluationVerdict::Allow);
        assert!(outcome.matches.is_empty());
        assert!(outcome.diagnostics.is_empty());
    }

    #[test]
    fn commit_trailer_rule_skips_message_option_values() {
        let artifact = CompiledRulesArtifact::new(99, vec![forbidden_trailer_rule()]);
        let input = EvaluationInput {
            command: "git commit -m --trailer AI-generated-by=bot".to_string(),
        };

        let outcome = evaluate_artifact(&artifact, &input);

        assert_eq!(outcome.verdict, EvaluationVerdict::Allow);
        assert!(outcome.matches.is_empty());
        assert!(outcome.diagnostics.is_empty());
    }

    #[test]
    fn evaluation_error_fails_open_for_whole_artifact() {
        let artifact = CompiledRulesArtifact::new(
            99,
            vec![
                CompiledRule {
                    predicate: RulePredicate::CommandRegex {
                        pattern: "(".to_string(),
                        message: "broken".to_string(),
                    },
                    ..package_manager_rule(RuleAction::Block)
                },
                forbidden_trailer_rule(),
            ],
        );
        let input = EvaluationInput {
            command: "git commit --trailer AI-generated-by=bot".to_string(),
        };

        let outcome = evaluate_artifact(&artifact, &input);

        assert_eq!(outcome.verdict, EvaluationVerdict::Allow);
        assert!(outcome.matches.is_empty());
        assert_eq!(outcome.diagnostics.len(), 1);
        assert!(outcome.diagnostics[0].message.contains("invalid regex"));
    }
}
