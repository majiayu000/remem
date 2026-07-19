use std::collections::{HashMap, HashSet};

use brush_parser::ast::SimpleCommand;

use super::{
    AliasDefinition, CommandCollector, ExitTrapDefinition, FunctionDefinition, PositionalContext,
};

#[derive(Clone)]
pub(super) struct ShellStateSnapshot {
    functions: HashMap<String, Vec<FunctionDefinition>>,
    exported_functions: HashSet<String>,
    readonly_variables: HashSet<String>,
    active_functions: HashSet<String>,
    aliases: HashMap<String, Vec<AliasDefinition>>,
    pending_aliases: HashMap<String, Option<Vec<AliasDefinition>>>,
    pending_clear_aliases: bool,
    active_aliases: HashSet<String>,
    expand_aliases: bool,
    alias_expansion_active: bool,
    lastpipe: bool,
    nocasematch: bool,
    monitor_mode: bool,
    exit_traps: Vec<ExitTrapDefinition>,
    execution_terminated: bool,
    positional_execution_is_definite: bool,
    positional_set_generation: u64,
    last_positional_status: Option<bool>,
    last_positional_success: Option<PositionalContext>,
    last_positional_failure: Option<PositionalContext>,
    inherited_stdin: Option<String>,
    positional_context: Option<PositionalContext>,
}

impl CommandCollector {
    pub(super) fn collect_correlated_command_variants(
        &mut self,
        segments: &[Vec<String>],
        command: &SimpleCommand,
    ) -> Result<bool, String> {
        let Some(context) = self.positional_context.clone() else {
            return Ok(false);
        };
        if context.possible_arguments.is_empty() {
            return Ok(false);
        }
        let argument_sets = std::iter::once(context.arguments)
            .chain(context.possible_arguments)
            .collect::<Vec<_>>();
        if segments.len() != argument_sets.len() {
            return Ok(false);
        }
        let zero_argument = context.zero_argument;
        let alternatives = segments
            .iter()
            .cloned()
            .zip(argument_sets)
            .collect::<Vec<_>>();
        self.collect_alternative_shell_states(
            alternatives,
            true,
            |collector, (tokens, arguments)| {
                let direct_name = super::static_execution::direct_command_name(&tokens);
                let command_status = direct_name
                    .filter(|name| !collector.functions.contains_key(*name))
                    .and_then(|_| super::static_execution::static_builtin_command_name(&tokens))
                    .and_then(|name| match name {
                        ":" | "true" => Some(true),
                        "false" => Some(false),
                        _ => None,
                    });
                collector.positional_context = Some(PositionalContext {
                    zero_argument: zero_argument.clone(),
                    arguments,
                    possible_arguments: Vec::new(),
                });
                collector.collect_static_tokens(tokens, command)?;
                if collector.last_positional_success.is_none()
                    && collector.last_positional_failure.is_none()
                {
                    let context = collector.positional_context.clone();
                    match command_status {
                        Some(true) => collector.last_positional_success = context,
                        Some(false) => collector.last_positional_failure = context,
                        None => {
                            collector.last_positional_success = context.clone();
                            collector.last_positional_failure = context;
                        }
                    }
                    collector.last_positional_status = command_status;
                }
                Ok(())
            },
        )?;
        Ok(true)
    }

    pub(super) fn collect_alternative_shell_states<T>(
        &mut self,
        alternatives: Vec<T>,
        executes_on_all_paths: bool,
        mut collect: impl FnMut(&mut Self, T) -> Result<(), String>,
    ) -> Result<(), String> {
        let base = self.snapshot_shell_state();
        let saved_execution_is_definite = self.execution_is_definite;
        let saved_positional_execution_is_definite = self.positional_execution_is_definite;
        let mut outcomes = Vec::with_capacity(alternatives.len() + 1);
        for alternative in alternatives {
            self.restore_shell_state(base.clone());
            self.execution_is_definite = true;
            self.positional_execution_is_definite = true;
            if let Err(error) = collect(self, alternative) {
                self.restore_shell_state(base);
                self.execution_is_definite = saved_execution_is_definite;
                self.positional_execution_is_definite = saved_positional_execution_is_definite;
                return Err(error);
            }
            if self.execution_terminated {
                if let Err(error) = self.collect_exit_traps() {
                    self.restore_shell_state(base);
                    self.execution_is_definite = saved_execution_is_definite;
                    self.positional_execution_is_definite = saved_positional_execution_is_definite;
                    return Err(error);
                }
            }
            outcomes.push(self.snapshot_shell_state());
        }
        if !executes_on_all_paths || !saved_execution_is_definite {
            outcomes.push(base.clone());
        }
        self.restore_shell_state(merge_shell_state_snapshots(outcomes).unwrap_or(base));
        self.execution_is_definite = saved_execution_is_definite;
        self.positional_execution_is_definite = saved_positional_execution_is_definite;
        Ok(())
    }

    pub(super) fn snapshot_shell_state(&self) -> ShellStateSnapshot {
        ShellStateSnapshot {
            functions: self.functions.clone(),
            exported_functions: self.exported_functions.clone(),
            readonly_variables: self.readonly_variables.clone(),
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
            positional_execution_is_definite: self.positional_execution_is_definite,
            positional_set_generation: self.positional_set_generation,
            last_positional_status: self.last_positional_status,
            last_positional_success: self.last_positional_success.clone(),
            last_positional_failure: self.last_positional_failure.clone(),
            inherited_stdin: self.inherited_stdin.clone(),
            positional_context: self.positional_context.clone(),
        }
    }

    pub(super) fn restore_shell_state(&mut self, saved: ShellStateSnapshot) {
        self.functions = saved.functions;
        self.exported_functions = saved.exported_functions;
        self.readonly_variables = saved.readonly_variables;
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
        self.positional_execution_is_definite = saved.positional_execution_is_definite;
        self.positional_set_generation = saved.positional_set_generation;
        self.last_positional_status = saved.last_positional_status;
        self.last_positional_success = saved.last_positional_success;
        self.last_positional_failure = saved.last_positional_failure;
        self.inherited_stdin = saved.inherited_stdin;
        self.positional_context = saved.positional_context;
    }
}

fn merge_shell_state_snapshots(mut states: Vec<ShellStateSnapshot>) -> Option<ShellStateSnapshot> {
    if states.iter().any(|state| !state.execution_terminated) {
        states.retain(|state| !state.execution_terminated);
    }
    let mut merged = states.pop()?;
    for state in states {
        merged.functions = merge_function_maps(merged.functions, state.functions);
        merged.aliases = merge_alias_maps(merged.aliases, state.aliases);
        merged.pending_aliases =
            merge_pending_alias_maps(merged.pending_aliases, state.pending_aliases);
        merged.exported_functions.extend(state.exported_functions);
        merged.readonly_variables.extend(state.readonly_variables);
        merged
            .active_functions
            .retain(|name| state.active_functions.contains(name));
        merged
            .active_aliases
            .retain(|name| state.active_aliases.contains(name));
        merged.pending_clear_aliases &= state.pending_clear_aliases;
        merged.expand_aliases |= state.expand_aliases;
        merged.alias_expansion_active |= state.alias_expansion_active;
        merged.lastpipe |= state.lastpipe;
        merged.nocasematch |= state.nocasematch;
        merged.monitor_mode &= state.monitor_mode;
        merged.exit_traps = merge_exit_traps(merged.exit_traps, state.exit_traps);
        merged.execution_terminated &= state.execution_terminated;
        merged.positional_execution_is_definite &= state.positional_execution_is_definite;
        merged.positional_set_generation = merged
            .positional_set_generation
            .max(state.positional_set_generation);
        if merged.last_positional_status != state.last_positional_status {
            merged.last_positional_status = None;
        }
        merged.last_positional_success = super::shell_state::merge_positional_contexts(
            merged.last_positional_success,
            state.last_positional_success,
        );
        merged.last_positional_failure = super::shell_state::merge_positional_contexts(
            merged.last_positional_failure,
            state.last_positional_failure,
        );
        if merged.inherited_stdin != state.inherited_stdin {
            merged.inherited_stdin = None;
        }
        merged.positional_context = super::shell_state::merge_positional_contexts(
            merged.positional_context,
            state.positional_context,
        );
    }
    Some(merged)
}

fn merge_function_maps(
    mut left: HashMap<String, Vec<FunctionDefinition>>,
    right: HashMap<String, Vec<FunctionDefinition>>,
) -> HashMap<String, Vec<FunctionDefinition>> {
    let left_definite = definite_names(&left);
    let right_definite = definite_names(&right);
    for (name, definitions) in right {
        let entry = left.entry(name).or_default();
        for definition in definitions {
            let body = definition.body.to_string();
            if !entry.iter().any(|current| current.body.to_string() == body) {
                entry.push(definition);
            }
        }
    }
    for (name, definitions) in &mut left {
        for definition in definitions.iter_mut() {
            definition.is_definite = false;
        }
        if left_definite.contains(name) && right_definite.contains(name) {
            if let Some(definition) = definitions.first_mut() {
                definition.is_definite = true;
            }
        }
    }
    left
}

fn merge_alias_maps(
    mut left: HashMap<String, Vec<AliasDefinition>>,
    right: HashMap<String, Vec<AliasDefinition>>,
) -> HashMap<String, Vec<AliasDefinition>> {
    let left_definite = definite_names(&left);
    let right_definite = definite_names(&right);
    let left_expandable = definitely_expandable_names(&left);
    let right_expandable = definitely_expandable_names(&right);
    for (name, definitions) in right {
        let entry = left.entry(name).or_default();
        for definition in definitions {
            if let Some(current) = entry
                .iter_mut()
                .find(|current| current.payload == definition.payload)
            {
                current.is_expandable |= definition.is_expandable;
            } else {
                entry.push(definition);
            }
        }
    }
    for (name, definitions) in &mut left {
        for definition in definitions.iter_mut() {
            definition.is_definite = false;
            definition.is_definitely_expandable = false;
        }
        if left_definite.contains(name) && right_definite.contains(name) {
            if let Some(definition) = definitions.first_mut() {
                definition.is_definite = true;
            }
        }
        if left_expandable.contains(name) && right_expandable.contains(name) {
            if let Some(definition) = definitions.first_mut() {
                definition.is_definitely_expandable = true;
            }
        }
    }
    left
}

fn merge_pending_alias_maps(
    mut left: HashMap<String, Option<Vec<AliasDefinition>>>,
    right: HashMap<String, Option<Vec<AliasDefinition>>>,
) -> HashMap<String, Option<Vec<AliasDefinition>>> {
    for (name, right_definitions) in right {
        match (left.remove(&name).flatten(), right_definitions) {
            (Some(left_definitions), Some(right_definitions)) => {
                let maps = HashMap::from([(name.clone(), left_definitions)]);
                let right_map = HashMap::from([(name.clone(), right_definitions)]);
                let mut merged = merge_alias_maps(maps, right_map);
                left.insert(name.clone(), merged.remove(&name));
            }
            (Some(mut definitions), None) | (None, Some(mut definitions)) => {
                definitions
                    .iter_mut()
                    .for_each(|definition| definition.is_definite = false);
                left.insert(name, Some(definitions));
            }
            (None, None) => {
                left.insert(name, None);
            }
        }
    }
    left
}

fn merge_exit_traps(
    mut left: Vec<ExitTrapDefinition>,
    right: Vec<ExitTrapDefinition>,
) -> Vec<ExitTrapDefinition> {
    let left_payloads = left
        .iter()
        .filter(|trap| trap.is_definite)
        .map(|trap| trap.payload.clone())
        .collect::<HashSet<_>>();
    let right_payloads = right
        .iter()
        .filter(|trap| trap.is_definite)
        .map(|trap| trap.payload.clone())
        .collect::<HashSet<_>>();
    for trap in right {
        if !left.iter().any(|current| current.payload == trap.payload) {
            left.push(trap);
        }
    }
    for trap in &mut left {
        trap.is_definite =
            left_payloads.contains(&trap.payload) && right_payloads.contains(&trap.payload);
    }
    left
}

fn definite_names<T>(definitions: &HashMap<String, Vec<T>>) -> HashSet<String>
where
    T: DefinitionCertainty,
{
    definitions
        .iter()
        .filter(|(_, definitions)| definitions.iter().any(DefinitionCertainty::is_definite))
        .map(|(name, _)| name.clone())
        .collect()
}

fn definitely_expandable_names(
    definitions: &HashMap<String, Vec<AliasDefinition>>,
) -> HashSet<String> {
    definitions
        .iter()
        .filter(|(_, definitions)| {
            definitions
                .iter()
                .any(|definition| definition.is_definitely_expandable)
        })
        .map(|(name, _)| name.clone())
        .collect()
}

trait DefinitionCertainty {
    fn is_definite(&self) -> bool;
}

impl DefinitionCertainty for FunctionDefinition {
    fn is_definite(&self) -> bool {
        self.is_definite
    }
}

impl DefinitionCertainty for AliasDefinition {
    fn is_definite(&self) -> bool {
        self.is_definite
    }
}
