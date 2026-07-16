use super::static_execution::{
    direct_command_name, static_alias_definitions, static_exit_trap_change,
    static_shopt_expand_aliases, static_shopt_lastpipe, static_unalias_names, ExitTrapChange,
};
use super::{unwrap, AliasDefinition, CommandCollector, ExitTrapDefinition};

impl CommandCollector {
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
        if let Some(definitions) = static_alias_definitions(tokens) {
            for (name, payload) in definitions {
                if self.execution_is_definite {
                    self.aliases.insert(
                        name.to_string(),
                        vec![AliasDefinition {
                            payload: payload.to_string(),
                            is_definite: true,
                        }],
                    );
                } else {
                    let definitions = self.aliases.entry(name.to_string()).or_default();
                    definitions
                        .iter_mut()
                        .for_each(|definition| definition.is_definite = false);
                    definitions.push(AliasDefinition {
                        payload: payload.to_string(),
                        is_definite: false,
                    });
                }
            }
        }
        if let Some(names) = static_unalias_names(tokens) {
            if names.is_empty() && self.execution_is_definite {
                self.aliases.clear();
            }
            for name in names {
                if self.execution_is_definite {
                    self.aliases.remove(name);
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
        if !self.expand_aliases {
            return Ok(false);
        }
        let Some(name) = direct_command_name(tokens) else {
            return Ok(false);
        };
        let Some(definitions) = self.aliases.get(name).cloned() else {
            return Ok(false);
        };
        let definitely_defined = definitions.iter().any(|definition| definition.is_definite);
        if !self.active_aliases.insert(name.to_string()) {
            return Ok(definitely_defined);
        }
        let command_index = unwrap::direct_command_index(tokens).expect("direct alias command");
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

    pub(super) fn collect_exit_traps(&mut self) -> Result<(), String> {
        let traps = std::mem::take(&mut self.exit_traps);
        for trap in traps {
            self.with_execution_certainty(trap.is_definite, |collector| {
                collector.collect_source(&trap.payload)
            })?;
        }
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
            self.expand_aliases = false;
            self.lastpipe = false;
        }
        self.active_functions.clear();
        self.active_aliases.clear();
        self.exit_traps.clear();
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
        self.active_aliases.clear();
        self.expand_aliases = false;
        self.lastpipe = false;
        self.exit_traps.clear();
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
            active_aliases: self.active_aliases.clone(),
            expand_aliases: self.expand_aliases,
            lastpipe: self.lastpipe,
            exit_traps: self.exit_traps.clone(),
            inherited_stdin: self.inherited_stdin.clone(),
        }
    }

    fn restore_shell_state(&mut self, saved: ShellStateSnapshot) {
        self.functions = saved.functions;
        self.exported_functions = saved.exported_functions;
        self.active_functions = saved.active_functions;
        self.aliases = saved.aliases;
        self.active_aliases = saved.active_aliases;
        self.expand_aliases = saved.expand_aliases;
        self.lastpipe = saved.lastpipe;
        self.exit_traps = saved.exit_traps;
        self.inherited_stdin = saved.inherited_stdin;
    }
}

struct ShellStateSnapshot {
    functions: std::collections::HashMap<String, Vec<super::FunctionDefinition>>,
    exported_functions: std::collections::HashSet<String>,
    active_functions: std::collections::HashSet<String>,
    aliases: std::collections::HashMap<String, Vec<AliasDefinition>>,
    active_aliases: std::collections::HashSet<String>,
    expand_aliases: bool,
    lastpipe: bool,
    exit_traps: Vec<ExitTrapDefinition>,
    inherited_stdin: Option<String>,
}

fn quote_alias_argument(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}
