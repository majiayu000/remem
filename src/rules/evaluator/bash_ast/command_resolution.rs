use brush_parser::ast::{CommandPrefixOrSuffixItem, SimpleCommand};

use super::function_args::expand_function_body;
use super::static_execution::{
    direct_command_name, static_builtin_command_name, static_env_split_tokens, static_eval_payload,
    static_export_function_change, static_shell_command_payload, static_shell_exits,
    static_shell_is_bash, static_shell_reads_stdin, static_source_stdin_arguments,
    static_token_measure, static_unset_function_names,
};
use super::stdin_payload::EffectiveStdin;
use super::{unwrap, CommandCollector, PositionalContext};

impl CommandCollector {
    pub(super) fn collect_static_tokens(
        &mut self,
        mut tokens: Vec<String>,
        command: &SimpleCommand,
    ) -> Result<(), String> {
        while let Some(expanded) = static_env_split_tokens(&tokens) {
            let before = static_token_measure(&tokens);
            let after = static_token_measure(&expanded);
            if after >= before {
                return Err("env -S static argv expansion did not make progress".to_string());
            }
            tokens = expanded;
        }
        if command_has_fallible_setup(command) {
            return self.collect_alternative_shell_states(
                vec![true, false],
                true,
                |collector, setup_succeeded| {
                    if setup_succeeded {
                        collector
                            .collect_static_tokens_after_redirect_setup(tokens.clone(), command)?;
                        Ok(())
                    } else {
                        collector.last_positional_status = Some(false);
                        collector.last_positional_success = None;
                        collector.last_positional_failure = collector.positional_context.clone();
                        Ok(())
                    }
                },
            );
        }
        self.collect_static_tokens_after_redirect_setup(tokens, command)
    }

    fn collect_static_tokens_after_redirect_setup(
        &mut self,
        tokens: Vec<String>,
        command: &SimpleCommand,
    ) -> Result<(), String> {
        if self.collect_static_alias_call(&tokens, command)? {
            return Ok(());
        }
        self.collect_static_tokens_after_alias(tokens, command)
    }

    pub(super) fn collect_static_tokens_after_alias(
        &mut self,
        tokens: Vec<String>,
        command: &SimpleCommand,
    ) -> Result<(), String> {
        if self.collect_static_function_call(&tokens, command)? {
            return Ok(());
        }
        self.collect_static_tokens_after_function(tokens, command)
    }

    pub(super) fn collect_static_tokens_after_function(
        &mut self,
        tokens: Vec<String>,
        command: &SimpleCommand,
    ) -> Result<(), String> {
        if let Some(names) = static_unset_function_names(&tokens) {
            for name in names {
                if self.execution_is_definite {
                    self.functions.remove(name);
                    self.exported_functions.remove(name);
                } else if let Some(definitions) = self.functions.get_mut(name) {
                    for definition in definitions {
                        definition.is_definite = false;
                    }
                }
            }
        }
        if let Some((exported, names)) = static_export_function_change(&tokens) {
            for name in names {
                if exported && self.functions.contains_key(name) {
                    self.exported_functions.insert(name.to_string());
                } else if !exported && self.execution_is_definite {
                    self.exported_functions.remove(name);
                }
            }
        }
        self.apply_static_shell_state(&tokens);
        self.apply_static_positional_state(&tokens, false);
        if let Some(payload) = static_eval_payload(&tokens) {
            self.collect_source(&payload)?;
        }
        if let Some(shell_command) = static_shell_command_payload(&tokens) {
            let inherited_stdin = match self.effective_stdin_payload(command)? {
                EffectiveStdin::Replaced(payload) => payload,
                EffectiveStdin::Untouched => None,
            };
            self.with_child_shell_scope(static_shell_is_bash(&tokens), |collector| {
                collector.with_inherited_stdin(inherited_stdin, |collector| {
                    collector.with_positional_context(
                        Some(PositionalContext {
                            zero_argument: shell_command.zero_argument,
                            arguments: shell_command.arguments,
                            possible_arguments: Vec::new(),
                        }),
                        |collector| {
                            collector.collect_source(&shell_command.payload)?;
                            collector.collect_exit_traps()
                        },
                    )
                })
            })?;
        }
        if static_shell_reads_stdin(&tokens) {
            let payload = match self.effective_stdin_payload(command)? {
                EffectiveStdin::Replaced(payload) => payload,
                EffectiveStdin::Untouched => self.inherited_stdin.clone(),
            };
            if let Some(payload) = payload {
                self.with_child_shell_scope(static_shell_is_bash(&tokens), |collector| {
                    collector.collect_source(&payload)
                })?;
            }
        } else if let Some(arguments) = static_source_stdin_arguments(&tokens) {
            self.collect_source_stdin_payload(command, arguments)?;
        }
        self.record_static_command_outcomes(&tokens);
        let shell_exits = static_shell_exits(&tokens);
        self.segments.push(tokens);
        if self.execution_is_definite && shell_exits {
            self.execution_terminated = true;
        }
        Ok(())
    }

    fn record_static_command_outcomes(&mut self, tokens: &[String]) {
        if self.positional_context.is_none() {
            return;
        }
        if self.last_positional_success.is_some() || self.last_positional_failure.is_some() {
            return;
        }
        let status = static_builtin_command_name(tokens).and_then(|name| match name {
            ":" | "true" => Some(true),
            "false" => Some(false),
            _ => None,
        });
        let context = self.positional_context.clone();
        match status {
            Some(true) => self.last_positional_success = context,
            Some(false) => self.last_positional_failure = context,
            None => {
                self.last_positional_success = context.clone();
                self.last_positional_failure = context;
            }
        }
        self.last_positional_status = status;
    }

    fn collect_static_function_call(
        &mut self,
        tokens: &[String],
        command: &SimpleCommand,
    ) -> Result<bool, String> {
        let Some(name) = direct_command_name(tokens) else {
            return Ok(false);
        };
        let Some(definitions) = self.functions.get(name).cloned() else {
            return Ok(false);
        };
        let definitely_defined = definitions.iter().any(|definition| definition.is_definite);
        if !self.active_functions.insert(name.to_string()) {
            self.collect_static_tokens_after_function(tokens.to_vec(), command)?;
            return Ok(true);
        }
        let result = (|| {
            let command_index = unwrap::direct_command_index(tokens)
                .ok_or_else(|| "function call lost its command position".to_string())?;
            let arguments = tokens[command_index + 1..].to_vec();
            let mut alternatives = definitions.into_iter().map(Some).collect::<Vec<_>>();
            if !definitely_defined {
                alternatives.push(None);
            }
            self.collect_alternative_shell_states(alternatives, true, |collector, definition| {
                let Some(definition) = definition else {
                    return collector
                        .collect_static_tokens_after_function(tokens.to_vec(), command);
                };
                let source = expand_function_body(&definition.body, &arguments);
                let function_context =
                    collector
                        .positional_context
                        .as_ref()
                        .map(|context| PositionalContext {
                            zero_argument: context.zero_argument.clone(),
                            arguments: arguments.clone(),
                            possible_arguments: Vec::new(),
                        });
                collector.with_positional_context(function_context, |collector| {
                    collector.collect_source(&source)
                })?;
                let success = collector.last_positional_success.is_some();
                let failure = collector.last_positional_failure.is_some();
                collector.last_positional_success = success
                    .then(|| collector.positional_context.clone())
                    .flatten();
                collector.last_positional_failure = failure
                    .then(|| collector.positional_context.clone())
                    .flatten();
                Ok(())
            })?;
            Ok(())
        })();
        self.active_functions.remove(name);
        result.map(|()| true)
    }
}

pub(super) fn command_has_fallible_setup(command: &SimpleCommand) -> bool {
    command.prefix.is_some()
        || command
            .prefix
            .iter()
            .flat_map(|items| &items.0)
            .chain(command.suffix.iter().flat_map(|items| &items.0))
            .any(|item| matches!(item, CommandPrefixOrSuffixItem::IoRedirect(_)))
}
