use brush_parser::ast::{SimpleCommand, Word};

use super::function_args::{
    bare_shell_positional_fields, expand_shell_command, has_shell_positional_reference,
};
use super::static_words::{
    critical_brace_variants, expand_brace_pieces, static_word_pieces, StaticExpansionError,
};
use super::{CommandCollector, PositionalContext, DYNAMIC_SHELL_WORD, MAX_STATIC_WORD_VARIANTS};

impl CommandCollector {
    pub(super) fn collect_possible_positional_command_variants(
        &mut self,
        command: &SimpleCommand,
    ) -> Result<bool, String> {
        let Some(context) = self
            .positional_context
            .clone()
            .filter(|context| !context.possible_arguments.is_empty())
        else {
            return Ok(false);
        };
        let source = command.to_string();
        if !positional_command_varies(&source, &self.options, &context)? {
            return Ok(false);
        }
        let mut alternatives = context.possible_arguments.clone();
        bound_possible_positional_arguments(&context.arguments, &mut alternatives);
        alternatives.insert(0, context.arguments.clone());
        let mut outcomes = Vec::new();
        for arguments in alternatives {
            self.positional_context = Some(PositionalContext {
                zero_argument: context.zero_argument.clone(),
                arguments,
                possible_arguments: Vec::new(),
            });
            if let Err(error) = self.with_execution_certainty(false, |collector| {
                collector.collect_simple_command(command)
            }) {
                self.positional_context = Some(context);
                return Err(error);
            }
            if let Some(outcome) = &self.positional_context {
                outcomes.push(outcome.arguments.clone());
                outcomes.extend(outcome.possible_arguments.iter().cloned());
            }
        }
        let current = context.arguments;
        bound_possible_positional_arguments(&current, &mut outcomes);
        self.positional_context = Some(PositionalContext {
            zero_argument: context.zero_argument,
            arguments: current,
            possible_arguments: outcomes,
        });
        Ok(true)
    }

    pub(super) fn command_word_variants(&self, word: &Word) -> Result<Vec<String>, String> {
        if let Some(context) = &self.positional_context {
            if let Some(fields) = shell_positional_variant_fields(
                &word.value,
                &self.options,
                context.zero_argument.as_deref(),
                &context.arguments,
                &context.possible_arguments,
            )? {
                return Ok(fields);
            }
        }
        let source = self.expand_positional_source(&word.value)?;
        let Some(brace_pieces) = brush_parser::word::parse_brace_expansions(&source, &self.options)
            .map_err(|error| format!("Bash brace expansion parse error: {error}"))?
        else {
            return Ok(vec![self.command_word(word)?]);
        };
        let expanded = match expand_brace_pieces(&brace_pieces) {
            Ok(expanded) => expanded,
            Err(StaticExpansionError::Limit) => {
                let mut variants = critical_brace_variants(&brace_pieces);
                variants.truncate(MAX_STATIC_WORD_VARIANTS - 1);
                variants.push(DYNAMIC_SHELL_WORD.to_string());
                return Ok(variants);
            }
            Err(StaticExpansionError::Invalid(message)) => return Err(message),
        };
        expanded
            .into_iter()
            .map(|value| {
                let pieces = brush_parser::word::parse(&value, &self.options)
                    .map_err(|error| format!("Bash expanded word parse error: {error}"))?;
                Ok(static_word_pieces(&pieces).unwrap_or_else(|| DYNAMIC_SHELL_WORD.to_string()))
            })
            .collect()
    }
}

pub(super) fn bound_possible_positional_arguments(
    current: &[String],
    possible: &mut Vec<Vec<String>>,
) {
    let mut unique = Vec::new();
    for arguments in possible.drain(..) {
        if arguments != current && !unique.contains(&arguments) {
            unique.push(arguments);
        }
    }
    unique.sort_by_key(|arguments| !positional_arguments_are_critical(arguments));
    unique.truncate(MAX_STATIC_WORD_VARIANTS - 1);
    *possible = unique;
}

fn shell_positional_variant_fields(
    source: &str,
    options: &brush_parser::ParserOptions,
    zero_argument: Option<&str>,
    arguments: &[String],
    possible_arguments: &[Vec<String>],
) -> Result<Option<Vec<String>>, String> {
    let mut combined = Vec::new();
    for arguments in std::iter::once(arguments).chain(possible_arguments.iter().map(Vec::as_slice))
    {
        let Some(fields) = bare_shell_positional_fields(source, options, zero_argument, arguments)?
        else {
            return Ok(None);
        };
        combined.extend(fields);
    }
    Ok(Some(combined))
}

fn positional_command_varies(
    source: &str,
    options: &brush_parser::ParserOptions,
    context: &PositionalContext,
) -> Result<bool, String> {
    if !has_shell_positional_reference(source) {
        return Ok(false);
    }
    let current = expand_shell_command(
        source,
        options,
        context.zero_argument.as_deref(),
        &context.arguments,
    )?;
    for arguments in &context.possible_arguments {
        let possible =
            expand_shell_command(source, options, context.zero_argument.as_deref(), arguments)?;
        if possible != current {
            return Ok(true);
        }
    }
    Ok(false)
}

fn positional_arguments_are_critical(arguments: &[String]) -> bool {
    arguments.iter().any(|argument| {
        let value = argument.to_ascii_lowercase();
        [
            "git", "push", "force", "mirror", "bash", "dash", "ksh", "sh", "zsh", "env", "exec",
            "eval", "trap", "alias", "unset", "set", "shift", "command", "builtin",
        ]
        .iter()
        .any(|critical| value.contains(critical))
    })
}
