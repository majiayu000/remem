use super::static_execution::{
    direct_command_name, static_alias_definitions, static_exit_trap_change, static_monitor_mode,
    static_set_positional_arguments, static_shopt_expand_aliases, static_shopt_lastpipe,
    static_shopt_nocasematch, static_unalias_names, ExitTrapChange,
};
use super::{unwrap, AliasDefinition, CommandCollector, ExitTrapDefinition, PositionalContext};

impl CommandCollector {
    pub(super) fn apply_static_positional_state(
        &mut self,
        tokens: &[String],
        resolves_to_function: bool,
    ) {
        let Some(arguments) = (!resolves_to_function)
            .then(|| static_set_positional_arguments(tokens))
            .flatten()
        else {
            return;
        };
        if self.execution_is_definite {
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
        } else {
            self.positional_context = Some(PositionalContext {
                zero_argument: None,
                arguments: Vec::new(),
                possible_arguments: vec![arguments.to_vec()],
            });
        }
    }

    pub(super) fn apply_static_shell_state(&mut self, tokens: &[String]) {
        if let Some(enabled) = static_shopt_expand_aliases(tokens) {
            if enabled || self.execution_is_definite {
                self.expand_aliases = enabled;
            }
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
                        }]),
                    );
                } else {
                    let mut definitions = self.aliases.get(name).cloned().unwrap_or_default();
                    definitions
                        .iter_mut()
                        .for_each(|definition| definition.is_definite = false);
                    definitions.push(AliasDefinition {
                        payload: payload.to_string(),
                        is_definite: false,
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

    pub(super) fn collect_static_alias_call(&mut self, tokens: &[String]) -> Result<bool, String> {
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
        let definitely_defined = definitions.iter().any(|definition| definition.is_definite);
        if !self.active_aliases.insert(name.to_string()) {
            return Ok(definitely_defined);
        }
        let result = (|| {
            for definition in definitions {
                let mut source = definition.payload;
                for argument in &tokens[command_index + 1..] {
                    source.push(' ');
                    source.push_str(&quote_alias_argument(argument));
                }
                self.with_execution_certainty(definition.is_definite, |collector| {
                    collector.collect_source(&source)
                })?;
            }
            Ok(())
        })();
        self.active_aliases.remove(name);
        result.map(|()| definitely_defined)
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
        self.execution_is_definite = saved && definitely_executes;
        let result = collect(self);
        self.execution_is_definite = saved;
        result
    }

    fn snapshot_shell_state(&self) -> ShellStateSnapshot {
        ShellStateSnapshot {
            functions: self.functions.clone(),
            exported_functions: self.exported_functions.clone(),
            active_functions: self.active_functions.clone(),
            aliases: self.aliases.clone(),
            pending_aliases: self.pending_aliases.clone(),
            pending_clear_aliases: self.pending_clear_aliases,
            active_aliases: self.active_aliases.clone(),
            expand_aliases: self.expand_aliases,
            alias_expansion_active: self.alias_expansion_active,
            lastpipe: self.lastpipe,
            nocasematch: self.nocasematch,
            monitor_mode: self.monitor_mode,
            exit_traps: self.exit_traps.clone(),
            execution_terminated: self.execution_terminated,
            inherited_stdin: self.inherited_stdin.clone(),
            positional_context: self.positional_context.clone(),
        }
    }

    fn restore_shell_state(&mut self, saved: ShellStateSnapshot) {
        self.functions = saved.functions;
        self.exported_functions = saved.exported_functions;
        self.active_functions = saved.active_functions;
        self.aliases = saved.aliases;
        self.pending_aliases = saved.pending_aliases;
        self.pending_clear_aliases = saved.pending_clear_aliases;
        self.active_aliases = saved.active_aliases;
        self.expand_aliases = saved.expand_aliases;
        self.alias_expansion_active = saved.alias_expansion_active;
        self.lastpipe = saved.lastpipe;
        self.nocasematch = saved.nocasematch;
        self.monitor_mode = saved.monitor_mode;
        self.exit_traps = saved.exit_traps;
        self.execution_terminated = saved.execution_terminated;
        self.inherited_stdin = saved.inherited_stdin;
        self.positional_context = saved.positional_context;
    }
}

struct ShellStateSnapshot {
    functions: std::collections::HashMap<String, Vec<super::FunctionDefinition>>,
    exported_functions: std::collections::HashSet<String>,
    active_functions: std::collections::HashSet<String>,
    aliases: std::collections::HashMap<String, Vec<AliasDefinition>>,
    pending_aliases: std::collections::HashMap<String, Option<Vec<AliasDefinition>>>,
    pending_clear_aliases: bool,
    active_aliases: std::collections::HashSet<String>,
    expand_aliases: bool,
    alias_expansion_active: bool,
    lastpipe: bool,
    nocasematch: bool,
    monitor_mode: bool,
    exit_traps: Vec<ExitTrapDefinition>,
    execution_terminated: bool,
    inherited_stdin: Option<String>,
    positional_context: Option<PositionalContext>,
}

fn quote_alias_argument(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}
