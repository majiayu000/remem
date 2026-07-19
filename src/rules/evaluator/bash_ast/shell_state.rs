use brush_parser::ast::{SimpleCommand, Word};

use super::function_args::{bare_shell_positional_fields, expand_shell_command};
use super::static_execution::{
    direct_command_name, static_alias_definitions, static_exit_trap_change, static_monitor_mode,
    static_positional_change, static_shopt_expand_aliases, static_shopt_lastpipe,
    static_shopt_nocasematch, static_unalias_names, ExitTrapChange, StaticPositionalChange,
};
use super::static_words::{append_word_variants, static_source_word_variants};
use super::{
    unwrap, AliasDefinition, CommandCollector, ExitTrapDefinition, PositionalContext,
    MAX_STATIC_WORD_VARIANTS,
};

impl CommandCollector {
    pub(super) fn append_command_name_variants(
        &self,
        segments: &mut Vec<Vec<String>>,
        word: &Word,
    ) -> Result<(), String> {
        if let Some(context) = &self.positional_context {
            let mut field_variants = Vec::new();
            for arguments in std::iter::once(context.arguments.as_slice())
                .chain(context.possible_arguments.iter().map(Vec::as_slice))
            {
                field_variants.push(self.positional_word_fields(word, context, arguments)?);
            }
            if field_variants.len() > 1 {
                let prefixes = std::mem::take(segments);
                for prefix in prefixes {
                    for fields in &field_variants {
                        let mut segment = prefix.clone();
                        segment.extend(fields.iter().cloned());
                        segments.push(segment);
                    }
                }
                return Ok(());
            }
        }
        append_word_variants(segments, self.command_word_variants(word)?);
        Ok(())
    }

    pub(super) fn append_command_argument_variants(
        &self,
        segments: &mut [Vec<String>],
        word: &Word,
    ) -> Result<(), String> {
        let Some(context) = &self.positional_context else {
            append_word_variants(segments, self.command_word_variants(word)?);
            return Ok(());
        };
        let context_count = 1 + context.possible_arguments.len();
        for (index, segment) in segments.iter_mut().enumerate() {
            let arguments = if index % context_count == 0 {
                context.arguments.as_slice()
            } else {
                &context.possible_arguments[index % context_count - 1]
            };
            segment.extend(self.positional_word_fields(word, context, arguments)?);
        }
        Ok(())
    }

    fn positional_word_fields(
        &self,
        word: &Word,
        context: &PositionalContext,
        arguments: &[String],
    ) -> Result<Vec<String>, String> {
        if let Some(fields) = bare_shell_positional_fields(
            &word.value,
            &self.options,
            context.zero_argument.as_deref(),
            arguments,
        )? {
            return Ok(fields);
        }
        let source = expand_shell_command(
            &word.value,
            &self.options,
            context.zero_argument.as_deref(),
            arguments,
        )?;
        static_source_word_variants(&source, &self.options)
    }

    pub(super) fn collect_correlated_positional_tokens(
        &mut self,
        segments: &[Vec<String>],
    ) -> bool {
        let Some(context) = self.positional_context.clone() else {
            return false;
        };
        if context.possible_arguments.is_empty() {
            return false;
        }
        let argument_sets = std::iter::once(context.arguments.clone())
            .chain(context.possible_arguments.clone())
            .collect::<Vec<_>>();
        if segments.len() != argument_sets.len() {
            return false;
        }
        let mut outputs = Vec::new();
        let mut successes = Vec::new();
        let mut failures = Vec::new();
        for (tokens, arguments) in segments.iter().zip(&argument_sets) {
            let Some(change) = static_positional_change(tokens) else {
                return false;
            };
            let name = direct_command_name(tokens).unwrap_or_default();
            let alias_resolves = unwrap::direct_command_index(tokens).is_some_and(|index| {
                self.alias_expansion_active
                    && self.aliases.contains_key(name)
                    && !unwrap::is_expanded_command_word(&tokens[index])
            });
            if self.functions.contains_key(name) || alias_resolves {
                return false;
            }
            let (output, success) = match change {
                StaticPositionalChange::Set(arguments) => (arguments.to_vec(), true),
                StaticPositionalChange::Shift(count) if count <= arguments.len() => {
                    (arguments[count..].to_vec(), true)
                }
                StaticPositionalChange::Shift(_) => (arguments.clone(), false),
            };
            outputs.push(output.clone());
            if success {
                successes.push(output);
            } else {
                failures.push(output);
            }
        }
        if !self.positional_execution_is_definite {
            outputs.extend(argument_sets);
        }
        let zero_argument = context.zero_argument;
        self.positional_context = positional_context_from_arguments(zero_argument.clone(), outputs);
        self.last_positional_success =
            positional_context_from_arguments(zero_argument.clone(), successes);
        self.last_positional_failure = positional_context_from_arguments(zero_argument, failures);
        self.last_positional_status = match (
            self.last_positional_success.is_some(),
            self.last_positional_failure.is_some(),
        ) {
            (true, false) => Some(true),
            (false, true) => Some(false),
            _ => None,
        };
        self.segments.extend(segments.iter().cloned());
        true
    }

    pub(super) fn take_positional_outcomes(
        &mut self,
        negated: bool,
        static_success: Option<bool>,
    ) -> (Option<PositionalContext>, Option<PositionalContext>) {
        let has_positional_outcomes =
            self.last_positional_success.is_some() || self.last_positional_failure.is_some();
        let mut success = self.last_positional_success.take();
        let mut failure = self.last_positional_failure.take();
        if !has_positional_outcomes {
            match static_success {
                Some(true) => success = self.positional_context.clone(),
                Some(false) => failure = self.positional_context.clone(),
                None => {
                    success = self.positional_context.clone();
                    failure = self.positional_context.clone();
                }
            }
        }
        self.last_positional_status = None;
        if negated && has_positional_outcomes {
            std::mem::swap(&mut success, &mut failure);
        }
        (success, failure)
    }

    pub(super) fn with_positional_branch_execution<T>(
        &mut self,
        definitely_executes: bool,
        collect: impl FnOnce(&mut Self) -> Result<T, String>,
    ) -> Result<T, String> {
        let saved = self.execution_is_definite;
        self.execution_is_definite = saved && definitely_executes;
        let result = collect(self);
        self.execution_is_definite = saved;
        result
    }

    pub(super) fn apply_static_positional_state(
        &mut self,
        tokens: &[String],
        resolves_to_function: bool,
    ) {
        let Some(change) = (!resolves_to_function)
            .then(|| static_positional_change(tokens))
            .flatten()
        else {
            return;
        };
        let arguments = match change {
            StaticPositionalChange::Set(arguments) => arguments,
            StaticPositionalChange::Shift(count) => {
                self.apply_static_shift(count);
                return;
            }
        };
        if self.positional_execution_is_definite {
            let zero_argument = self
                .positional_context
                .as_ref()
                .and_then(|context| context.zero_argument.clone());
            self.positional_context = Some(PositionalContext {
                zero_argument,
                arguments: arguments.to_vec(),
                possible_arguments: Vec::new(),
            });
        } else if let Some(context) = &mut self.positional_context {
            let arguments = arguments.to_vec();
            if arguments != context.arguments && !context.possible_arguments.contains(&arguments) {
                context.possible_arguments.push(arguments);
            }
            bound_possible_positional_arguments(
                &context.arguments,
                &mut context.possible_arguments,
            );
        } else {
            self.positional_context = Some(PositionalContext {
                zero_argument: None,
                arguments: Vec::new(),
                possible_arguments: vec![arguments.to_vec()],
            });
        }
        self.last_positional_status = Some(true);
        self.last_positional_success = self.positional_context.clone();
        self.last_positional_failure = None;
    }

    fn apply_static_shift(&mut self, count: usize) {
        let Some(context) = &mut self.positional_context else {
            self.last_positional_status = Some(count == 0);
            return;
        };
        let shift = |arguments: &[String]| {
            if count <= arguments.len() {
                arguments[count..].to_vec()
            } else {
                arguments.to_vec()
            }
        };
        let statuses = std::iter::once(context.arguments.as_slice())
            .chain(context.possible_arguments.iter().map(Vec::as_slice))
            .map(|arguments| count <= arguments.len())
            .collect::<Vec<_>>();
        self.last_positional_status = statuses
            .iter()
            .all(|status| *status == statuses[0])
            .then_some(statuses[0]);
        let zero_argument = context.zero_argument.clone();
        let argument_sets = std::iter::once(context.arguments.clone())
            .chain(context.possible_arguments.clone())
            .collect::<Vec<_>>();
        self.last_positional_success = positional_context_from_arguments(
            zero_argument.clone(),
            argument_sets
                .iter()
                .filter(|arguments| count <= arguments.len())
                .map(|arguments| shift(arguments))
                .collect(),
        );
        self.last_positional_failure = positional_context_from_arguments(
            zero_argument,
            argument_sets
                .into_iter()
                .filter(|arguments| count > arguments.len())
                .collect(),
        );
        if self.positional_execution_is_definite {
            context.arguments = shift(&context.arguments);
            context.possible_arguments = context
                .possible_arguments
                .iter()
                .map(|arguments| shift(arguments))
                .collect();
        } else {
            let shifted = std::iter::once(context.arguments.as_slice())
                .chain(context.possible_arguments.iter().map(Vec::as_slice))
                .map(shift)
                .collect::<Vec<_>>();
            for arguments in shifted {
                if arguments != context.arguments
                    && !context.possible_arguments.contains(&arguments)
                {
                    context.possible_arguments.push(arguments);
                }
            }
        }
        bound_possible_positional_arguments(&context.arguments, &mut context.possible_arguments);
    }

    pub(super) fn apply_static_shell_state(&mut self, tokens: &[String]) {
        if let Some(enabled) = static_shopt_expand_aliases(tokens) {
            if enabled || self.execution_is_definite {
                self.expand_aliases = enabled;
            }
            self.update_alias_expandability(enabled);
        }
        if let Some(enabled) = static_shopt_lastpipe(tokens) {
            if enabled || self.execution_is_definite {
                self.lastpipe = enabled;
            }
        }
        if let Some(enabled) = static_shopt_nocasematch(tokens) {
            if enabled || self.execution_is_definite {
                self.nocasematch = enabled;
            }
        }
        if let Some(enabled) = static_monitor_mode(tokens) {
            if self.execution_is_definite || !enabled {
                self.monitor_mode = enabled;
            }
        }
        if let Some(definitions) = static_alias_definitions(tokens) {
            for (name, payload) in definitions {
                if self.execution_is_definite {
                    self.pending_aliases.insert(
                        name.to_string(),
                        Some(vec![AliasDefinition {
                            payload: payload.to_string(),
                            is_definite: true,
                            is_expandable: self.expand_aliases,
                            is_definitely_expandable: self.expand_aliases,
                        }]),
                    );
                } else {
                    let mut definitions = self.aliases.get(name).cloned().unwrap_or_default();
                    if !definitions.iter().any(|definition| definition.is_definite) {
                        definitions
                            .iter_mut()
                            .for_each(|definition| definition.is_definite = false);
                    }
                    definitions.push(AliasDefinition {
                        payload: payload.to_string(),
                        is_definite: false,
                        is_expandable: self.expand_aliases,
                        is_definitely_expandable: false,
                    });
                    self.pending_aliases
                        .insert(name.to_string(), Some(definitions));
                }
            }
        }
        if let Some(names) = static_unalias_names(tokens) {
            if names.is_empty() && self.execution_is_definite {
                self.pending_clear_aliases = true;
            }
            for name in names {
                if self.execution_is_definite {
                    self.pending_aliases.insert(name.to_string(), None);
                } else if let Some(definitions) = self.aliases.get_mut(name) {
                    definitions
                        .iter_mut()
                        .for_each(|definition| definition.is_definite = false);
                }
            }
        }
        if let Some(change) = static_exit_trap_change(tokens) {
            match change {
                ExitTrapChange::Set(payload) if self.execution_is_definite => {
                    self.exit_traps = vec![ExitTrapDefinition {
                        payload: payload.to_string(),
                        is_definite: true,
                    }];
                }
                ExitTrapChange::Set(payload) => {
                    self.exit_traps
                        .iter_mut()
                        .for_each(|trap| trap.is_definite = false);
                    self.exit_traps.push(ExitTrapDefinition {
                        payload: payload.to_string(),
                        is_definite: false,
                    });
                }
                ExitTrapChange::Reset if self.execution_is_definite => self.exit_traps.clear(),
                ExitTrapChange::Reset => self
                    .exit_traps
                    .iter_mut()
                    .for_each(|trap| trap.is_definite = false),
            }
        }
    }

    pub(super) fn collect_static_alias_call(
        &mut self,
        tokens: &[String],
        command: &SimpleCommand,
    ) -> Result<bool, String> {
        if !self.alias_expansion_active {
            return Ok(false);
        }
        let Some(name) = direct_command_name(tokens) else {
            return Ok(false);
        };
        let command_index = unwrap::direct_command_index(tokens)
            .ok_or_else(|| "alias call lost its command position".to_string())?;
        if unwrap::is_expanded_command_word(&tokens[command_index]) {
            return Ok(false);
        }
        let Some(definitions) = self.aliases.get(name).cloned() else {
            return Ok(false);
        };
        let definitions = definitions
            .into_iter()
            .filter(|definition| definition.is_expandable)
            .collect::<Vec<_>>();
        if definitions.is_empty() {
            return Ok(false);
        }
        let definitely_defined = definitions.iter().any(|definition| definition.is_definite)
            && definitions
                .iter()
                .any(|definition| definition.is_definitely_expandable);
        if !self.active_aliases.insert(name.to_string()) {
            self.collect_static_tokens_after_alias(tokens.to_vec(), command)?;
            return Ok(true);
        }
        let mut alternatives = definitions.into_iter().map(Some).collect::<Vec<_>>();
        if !definitely_defined {
            alternatives.push(None);
        }
        let result =
            self.collect_alternative_shell_states(alternatives, true, |collector, definition| {
                let Some(definition) = definition else {
                    return collector.collect_static_tokens_after_alias(tokens.to_vec(), command);
                };
                let mut source = definition.payload;
                for argument in &tokens[command_index + 1..] {
                    source.push(' ');
                    source.push_str(&quote_alias_argument(argument));
                }
                collector.collect_source(&source)
            });
        self.active_aliases.remove(name);
        result.map(|()| true)
    }

    pub(super) fn commit_pending_alias_changes(&mut self) {
        if self.pending_clear_aliases {
            self.aliases.clear();
        }
        for (name, definitions) in std::mem::take(&mut self.pending_aliases) {
            match definitions {
                Some(definitions) => {
                    self.aliases.insert(name, definitions);
                }
                None => {
                    self.aliases.remove(&name);
                }
            }
        }
        self.pending_clear_aliases = false;
    }

    fn update_alias_expandability(&mut self, enabled: bool) {
        let update = |definition: &mut AliasDefinition| {
            if self.execution_is_definite {
                definition.is_expandable = enabled;
                definition.is_definitely_expandable = enabled;
            } else if enabled {
                definition.is_expandable = true;
                definition.is_definitely_expandable = false;
            } else {
                definition.is_definitely_expandable = false;
            }
        };
        self.aliases.values_mut().flatten().for_each(update);
        self.pending_aliases
            .values_mut()
            .filter_map(Option::as_mut)
            .flatten()
            .for_each(update);
    }

    pub(super) fn collect_exit_traps(&mut self) -> Result<(), String> {
        let traps = std::mem::take(&mut self.exit_traps);
        let terminated = self.execution_terminated;
        self.execution_terminated = false;
        for trap in traps {
            self.with_execution_certainty(trap.is_definite, |collector| {
                collector.collect_source(&trap.payload)
            })?;
        }
        self.execution_terminated = terminated;
        self.exit_traps.clear();
        Ok(())
    }

    pub(super) fn with_function_scope<T>(
        &mut self,
        inherit: bool,
        collect: impl FnOnce(&mut Self) -> Result<T, String>,
    ) -> Result<T, String> {
        let saved = self.snapshot_shell_state();
        if !inherit {
            self.functions.clear();
            self.exported_functions.clear();
            self.aliases.clear();
            self.pending_aliases.clear();
            self.pending_clear_aliases = false;
            self.expand_aliases = false;
            self.alias_expansion_active = false;
            self.lastpipe = false;
            self.nocasematch = false;
            self.monitor_mode = false;
        }
        self.active_functions.clear();
        self.active_aliases.clear();
        self.exit_traps.clear();
        self.execution_terminated = false;
        let result = collect(self).and_then(|value| {
            self.collect_exit_traps()?;
            Ok(value)
        });
        self.restore_shell_state(saved);
        result
    }

    pub(super) fn with_child_shell_scope<T>(
        &mut self,
        inherit_exported: bool,
        collect: impl FnOnce(&mut Self) -> Result<T, String>,
    ) -> Result<T, String> {
        let saved = self.snapshot_shell_state();
        self.positional_context = None;
        if inherit_exported {
            self.functions
                .retain(|name, _| self.exported_functions.contains(name));
            self.exported_functions
                .retain(|name| self.functions.contains_key(name));
        } else {
            self.functions.clear();
            self.exported_functions.clear();
        }
        self.active_functions.clear();
        self.aliases.clear();
        self.pending_aliases.clear();
        self.pending_clear_aliases = false;
        self.active_aliases.clear();
        self.expand_aliases = false;
        self.alias_expansion_active = false;
        self.lastpipe = false;
        self.nocasematch = false;
        self.monitor_mode = false;
        self.exit_traps.clear();
        self.execution_terminated = false;
        let result = collect(self).and_then(|value| {
            self.collect_exit_traps()?;
            Ok(value)
        });
        self.restore_shell_state(saved);
        result
    }

    pub(super) fn with_inherited_stdin<T>(
        &mut self,
        payload: Option<String>,
        collect: impl FnOnce(&mut Self) -> Result<T, String>,
    ) -> Result<T, String> {
        let saved = self.inherited_stdin.clone();
        self.inherited_stdin = payload;
        let result = collect(self);
        self.inherited_stdin = saved;
        result
    }

    pub(super) fn with_execution_certainty<T>(
        &mut self,
        definitely_executes: bool,
        collect: impl FnOnce(&mut Self) -> Result<T, String>,
    ) -> Result<T, String> {
        let saved = self.execution_is_definite;
        let saved_positional = self.positional_execution_is_definite;
        self.execution_is_definite = saved && definitely_executes;
        self.positional_execution_is_definite = saved_positional && definitely_executes;
        let result = collect(self);
        self.execution_is_definite = saved;
        self.positional_execution_is_definite = saved_positional;
        result
    }
}

fn quote_alias_argument(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn positional_context_from_arguments(
    zero_argument: Option<String>,
    mut argument_sets: Vec<Vec<String>>,
) -> Option<PositionalContext> {
    argument_sets.dedup();
    let arguments = argument_sets.first()?.clone();
    let mut possible_arguments = argument_sets.into_iter().skip(1).collect::<Vec<_>>();
    bound_possible_positional_arguments(&arguments, &mut possible_arguments);
    Some(PositionalContext {
        zero_argument,
        arguments,
        possible_arguments,
    })
}

pub(in crate::rules::evaluator) fn bound_possible_positional_arguments(
    current: &[String],
    possible_arguments: &mut Vec<Vec<String>>,
) {
    let mut unique = Vec::new();
    for arguments in possible_arguments.drain(..) {
        if arguments != current && !unique.contains(&arguments) {
            unique.push(arguments);
        }
    }
    unique.sort_by_key(|arguments| !positional_arguments_are_security_relevant(arguments));
    unique.truncate(MAX_STATIC_WORD_VARIANTS - 1);
    *possible_arguments = unique;
}

fn positional_arguments_are_security_relevant(arguments: &[String]) -> bool {
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

pub(super) fn merge_positional_contexts(
    left: Option<PositionalContext>,
    right: Option<PositionalContext>,
) -> Option<PositionalContext> {
    match (left, right) {
        (Some(left), Some(right)) => positional_context_from_arguments(
            left.zero_argument.or(right.zero_argument),
            std::iter::once(left.arguments)
                .chain(left.possible_arguments)
                .chain(std::iter::once(right.arguments))
                .chain(right.possible_arguments)
                .collect(),
        ),
        (left, right) => left.or(right),
    }
}
