use std::collections::{HashMap, HashSet};
use std::io::Cursor;

use brush_parser::ast::{
    AndOr, Command, CommandPrefixOrSuffixItem, CompoundCommand, CompoundList, ExtendedTestExpr,
    FunctionBody, IoFileRedirectTarget, IoRedirect, Pipeline, Program, SimpleCommand,
    UnexpandedArithmeticExpr, Word,
};
use brush_parser::word::{Parameter, ParameterExpr, WordPiece, WordPieceWithSource};
use brush_parser::{Parser, ParserOptions};

mod control_flow;
mod function_args;
mod shell_state;
mod static_execution;
mod static_words;
mod stdin_payload;
pub(super) mod unwrap;

use function_args::{expand_function_body, expand_heredoc, expand_shell_command};
use static_execution::{
    direct_command_name, static_env_split_tokens, static_eval_payload,
    static_export_function_change, static_shell_command_payload, static_shell_exits,
    static_shell_is_bash, static_shell_reads_stdin, static_source_reads_stdin,
    static_unset_function_names,
};
use static_words::{
    append_word_variants, critical_brace_variants, expand_brace_pieces, static_word_pieces,
    StaticExpansionError,
};
use stdin_payload::EffectiveStdin;

const DYNAMIC_SHELL_WORD: &str = "__remem_dynamic_shell_word__";
const MAX_STATIC_WORD_VARIANTS: usize = 256;

pub(super) fn command_segments(source: &str) -> Result<Vec<Vec<String>>, String> {
    let mut collector = CommandCollector {
        options: ParserOptions::default(),
        segments: Vec::new(),
        functions: HashMap::new(),
        exported_functions: HashSet::new(),
        active_functions: HashSet::new(),
        aliases: HashMap::new(),
        pending_aliases: HashMap::new(),
        pending_clear_aliases: false,
        active_aliases: HashSet::new(),
        expand_aliases: false,
        alias_expansion_active: false,
        lastpipe: false,
        nocasematch: false,
        monitor_mode: false,
        exit_traps: Vec::new(),
        execution_terminated: false,
        execution_is_definite: true,
        inherited_stdin: None,
        positional_context: None,
    };
    collector.collect_source(source)?;
    collector.collect_exit_traps()?;
    Ok(collector.segments)
}

struct CommandCollector {
    options: ParserOptions,
    segments: Vec<Vec<String>>,
    functions: HashMap<String, Vec<FunctionDefinition>>,
    exported_functions: HashSet<String>,
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
    execution_is_definite: bool,
    inherited_stdin: Option<String>,
    positional_context: Option<PositionalContext>,
}

#[derive(Clone)]
struct PositionalContext {
    zero_argument: Option<String>,
    arguments: Vec<String>,
}

#[derive(Clone)]
struct FunctionDefinition {
    body: FunctionBody,
    is_definite: bool,
}

#[derive(Clone)]
struct AliasDefinition {
    payload: String,
    is_definite: bool,
}

#[derive(Clone)]
struct ExitTrapDefinition {
    payload: String,
    is_definite: bool,
}

impl CommandCollector {
    fn collect_source(&mut self, source: &str) -> Result<(), String> {
        let mut parser = Parser::new(Cursor::new(source.as_bytes()), &self.options);
        let program = parser
            .parse_program()
            .map_err(|error| format!("Bash AST parse error: {error}"))?;
        self.collect_program(&program)
    }

    fn collect_program(&mut self, program: &Program) -> Result<(), String> {
        for command in &program.complete_commands {
            self.alias_expansion_active = self.expand_aliases;
            self.collect_list(command)?;
            self.commit_pending_alias_changes();
            if self.execution_terminated {
                break;
            }
        }
        Ok(())
    }

    fn collect_list(&mut self, list: &CompoundList) -> Result<(), String> {
        for item in &list.0 {
            if self.execution_terminated {
                break;
            }
            self.collect_pipeline(&item.0.first)?;
            if self.execution_terminated {
                break;
            }
            let mut static_success = self.static_pipeline_success(&item.0.first)?;
            for additional in &item.0.additional {
                if self.execution_terminated {
                    break;
                }
                match additional {
                    AndOr::And(_) if static_success == Some(false) => {}
                    AndOr::Or(_) if static_success == Some(true) => {}
                    AndOr::And(pipeline) => {
                        let definitely_executes = static_success == Some(true);
                        self.with_execution_certainty(definitely_executes, |collector| {
                            collector.collect_pipeline(pipeline)
                        })?;
                        static_success =
                            and_status(static_success, self.static_pipeline_success(pipeline)?);
                    }
                    AndOr::Or(pipeline) => {
                        let definitely_executes = static_success == Some(false);
                        self.with_execution_certainty(definitely_executes, |collector| {
                            collector.collect_pipeline(pipeline)
                        })?;
                        static_success =
                            or_status(static_success, self.static_pipeline_success(pipeline)?);
                    }
                }
            }
        }
        Ok(())
    }

    fn static_pipeline_success(&self, pipeline: &Pipeline) -> Result<Option<bool>, String> {
        let [Command::Simple(command)] = pipeline.seq.as_slice() else {
            return Ok(None);
        };
        if command.prefix.is_some() || command.suffix.is_some() {
            return Ok(None);
        }
        let Some(name) = &command.word_or_name else {
            return Ok(None);
        };
        let name = self.command_word(name)?;
        if name == DYNAMIC_SHELL_WORD || self.functions.contains_key(&name) {
            return Ok(None);
        }
        let success = match name.as_str() {
            ":" | "true" => Some(true),
            "false" => Some(false),
            _ => None,
        };
        Ok(success.map(|success| if pipeline.bang { !success } else { success }))
    }

    fn collect_pipeline(&mut self, pipeline: &Pipeline) -> Result<(), String> {
        if pipeline.seq.len() == 1 {
            return self.collect_command(&pipeline.seq[0]);
        }
        let last = pipeline.seq.len() - 1;
        for (index, command) in pipeline.seq.iter().enumerate() {
            if index == last && self.lastpipe && !self.monitor_mode {
                self.collect_command(command)?;
            } else {
                self.with_function_scope(true, |collector| collector.collect_command(command))?;
            }
        }
        Ok(())
    }

    fn collect_command(&mut self, command: &Command) -> Result<(), String> {
        match command {
            Command::Simple(simple) => self.collect_simple_command(simple),
            Command::Compound(compound, redirects) => {
                self.collect_compound_command(compound)?;
                if let Some(redirects) = redirects {
                    for redirect in &redirects.0 {
                        self.collect_redirect_commands(redirect)?;
                    }
                }
                Ok(())
            }
            Command::Function(function) => {
                let name = self.command_word(&function.fname)?;
                if name != DYNAMIC_SHELL_WORD {
                    if self.execution_is_definite {
                        self.functions.insert(
                            name,
                            vec![FunctionDefinition {
                                body: function.body.clone(),
                                is_definite: true,
                            }],
                        );
                    } else {
                        let definitions = self.functions.entry(name).or_default();
                        for definition in definitions.iter_mut() {
                            definition.is_definite = false;
                        }
                        definitions.push(FunctionDefinition {
                            body: function.body.clone(),
                            is_definite: false,
                        });
                    }
                }
                Ok(())
            }
            Command::ExtendedTest(test, redirects) => {
                self.collect_extended_test(&test.expr)?;
                if let Some(redirects) = redirects {
                    for redirect in &redirects.0 {
                        self.collect_redirect_commands(redirect)?;
                    }
                }
                Ok(())
            }
        }
    }

    fn collect_compound_command(&mut self, command: &CompoundCommand) -> Result<(), String> {
        match command {
            CompoundCommand::Arithmetic(command) => {
                self.collect_arithmetic_expression(&command.expr)
            }
            CompoundCommand::ArithmeticForClause(command) => {
                for expression in [
                    command.initializer.as_ref(),
                    command.condition.as_ref(),
                    command.updater.as_ref(),
                ]
                .into_iter()
                .flatten()
                {
                    self.collect_arithmetic_expression(expression)?;
                }
                self.with_execution_certainty(false, |collector| {
                    collector.collect_list(&command.body.list)
                })
            }
            CompoundCommand::BraceGroup(command) => self.collect_list(&command.list),
            CompoundCommand::Subshell(command) => {
                self.with_function_scope(true, |collector| collector.collect_list(&command.list))
            }
            CompoundCommand::ForClause(command) => {
                if let Some(values) = &command.values {
                    for value in values {
                        self.collect_word_commands(value)?;
                    }
                }
                self.with_execution_certainty(false, |collector| {
                    collector.collect_list(&command.body.list)
                })
            }
            CompoundCommand::CaseClause(command) => self.collect_case_clause(command),
            CompoundCommand::IfClause(command) => self.collect_if_clause(command),
            CompoundCommand::WhileClause(command) => self.collect_loop(command, false),
            CompoundCommand::UntilClause(command) => self.collect_loop(command, true),
            CompoundCommand::Coprocess(command) => {
                if let Some(name) = &command.name {
                    self.collect_word_commands(name)?;
                }
                self.with_function_scope(true, |collector| collector.collect_command(&command.body))
            }
        }
    }

    fn collect_simple_command(&mut self, command: &SimpleCommand) -> Result<(), String> {
        let Some(name) = &command.word_or_name else {
            return self.collect_command_items(command.prefix.as_ref().map(|prefix| &prefix.0));
        };
        let mut segments = vec![Vec::new()];
        if let Some(prefix) = &command.prefix {
            for item in &prefix.0 {
                match item {
                    CommandPrefixOrSuffixItem::AssignmentWord(_, word) => {
                        self.collect_word_commands(word)?;
                        let token = self.command_word(word)?;
                        for segment in &mut segments {
                            segment.push(token.clone());
                        }
                    }
                    _ => self.collect_command_item(item)?,
                }
            }
        }
        self.collect_word_commands(name)?;
        append_word_variants(&mut segments, self.command_word_variants(name)?);
        if let Some(suffix) = &command.suffix {
            for item in &suffix.0 {
                match item {
                    CommandPrefixOrSuffixItem::Word(word)
                    | CommandPrefixOrSuffixItem::AssignmentWord(_, word) => {
                        self.collect_word_commands(word)?;
                        append_word_variants(&mut segments, self.command_word_variants(word)?);
                    }
                    _ => self.collect_command_item(item)?,
                }
            }
        }
        for tokens in segments {
            self.collect_static_tokens(tokens, command)?;
        }
        Ok(())
    }

    fn collect_static_tokens(
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
        self.apply_static_shell_state(&tokens);
        let resolves_to_function =
            direct_command_name(&tokens).is_some_and(|name| self.functions.contains_key(name));
        if !resolves_to_function {
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
        if self.collect_static_alias_call(&tokens)? || self.collect_static_function_call(&tokens)? {
            return Ok(());
        }
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
        } else if static_source_reads_stdin(&tokens) {
            let payload = match self.effective_stdin_payload(command)? {
                EffectiveStdin::Replaced(payload) => payload,
                EffectiveStdin::Untouched => self.inherited_stdin.clone(),
            };
            if let Some(payload) = payload {
                self.collect_source(&payload)?;
            }
        }
        let shell_exits = static_shell_exits(&tokens);
        self.segments.push(tokens);
        if self.execution_is_definite && shell_exits {
            self.execution_terminated = true;
        }
        Ok(())
    }

    fn collect_static_function_call(&mut self, tokens: &[String]) -> Result<bool, String> {
        let Some(name) = direct_command_name(tokens) else {
            return Ok(false);
        };
        let Some(definitions) = self.functions.get(name).cloned() else {
            return Ok(false);
        };
        let definitely_defined = definitions.iter().any(|definition| definition.is_definite);
        if !self.active_functions.insert(name.to_string()) {
            return Ok(definitely_defined);
        }
        let result = (|| {
            let command_index = unwrap::direct_command_index(tokens)
                .ok_or_else(|| "function call lost its command position".to_string())?;
            let arguments = &tokens[command_index + 1..];
            for definition in definitions {
                let source = expand_function_body(&definition.body, arguments);
                self.with_execution_certainty(definition.is_definite, |collector| {
                    let function_context =
                        collector
                            .positional_context
                            .as_ref()
                            .map(|context| PositionalContext {
                                zero_argument: context.zero_argument.clone(),
                                arguments: Vec::new(),
                            });
                    collector.with_positional_context(function_context, |collector| {
                        collector.collect_source(&source)
                    })
                })?;
            }
            Ok(())
        })();
        self.active_functions.remove(name);
        result.map(|()| definitely_defined)
    }

    fn collect_command_items(
        &mut self,
        items: Option<&Vec<CommandPrefixOrSuffixItem>>,
    ) -> Result<(), String> {
        if let Some(items) = items {
            for item in items {
                self.collect_command_item(item)?;
            }
        }
        Ok(())
    }

    fn collect_command_item(&mut self, item: &CommandPrefixOrSuffixItem) -> Result<(), String> {
        match item {
            CommandPrefixOrSuffixItem::IoRedirect(redirect) => {
                self.collect_redirect_commands(redirect)
            }
            CommandPrefixOrSuffixItem::Word(word)
            | CommandPrefixOrSuffixItem::AssignmentWord(_, word) => {
                self.collect_word_commands(word)
            }
            CommandPrefixOrSuffixItem::ProcessSubstitution(_, command) => {
                self.with_function_scope(true, |collector| collector.collect_list(&command.list))
            }
        }
    }

    fn collect_redirect_commands(&mut self, redirect: &IoRedirect) -> Result<(), String> {
        match redirect {
            IoRedirect::HereDocument(_, here_doc) => {
                if !here_doc.requires_expansion {
                    return Ok(());
                }
                let pieces = brush_parser::word::parse_heredoc(&here_doc.doc.value, &self.options)
                    .map_err(|error| format!("Bash here-document parse error: {error}"))?;
                self.collect_word_pieces(&pieces)
            }
            IoRedirect::HereString(_, word) | IoRedirect::OutputAndError(word, _) => {
                self.collect_word_commands(word)
            }
            IoRedirect::File(_, _, target) => match target {
                IoFileRedirectTarget::Filename(word) | IoFileRedirectTarget::Duplicate(word) => {
                    self.collect_word_commands(word)
                }
                IoFileRedirectTarget::ProcessSubstitution(_, command) => self
                    .with_function_scope(true, |collector| collector.collect_list(&command.list)),
                IoFileRedirectTarget::Fd(_) => Ok(()),
            },
        }
    }

    fn collect_extended_test(&mut self, expression: &ExtendedTestExpr) -> Result<(), String> {
        match expression {
            ExtendedTestExpr::And(left, right) | ExtendedTestExpr::Or(left, right) => {
                self.collect_extended_test(left)?;
                self.collect_extended_test(right)
            }
            ExtendedTestExpr::Not(expression) | ExtendedTestExpr::Parenthesized(expression) => {
                self.collect_extended_test(expression)
            }
            ExtendedTestExpr::UnaryTest(_, word) => self.collect_word_commands(word),
            ExtendedTestExpr::BinaryTest(_, left, right) => {
                self.collect_word_commands(left)?;
                self.collect_word_commands(right)
            }
        }
    }

    fn collect_word_commands(&mut self, word: &Word) -> Result<(), String> {
        let source = self.expand_positional_source(&word.value);
        let pieces = brush_parser::word::parse(&source, &self.options)
            .map_err(|error| format!("Bash word parse error: {error}"))?;
        self.collect_word_pieces(&pieces)
    }

    fn collect_arithmetic_expression(
        &mut self,
        expression: &UnexpandedArithmeticExpr,
    ) -> Result<(), String> {
        let source = self.expand_positional_source(&expression.value);
        let pieces = brush_parser::word::parse(&source, &self.options)
            .map_err(|error| format!("Bash arithmetic word parse error: {error}"))?;
        self.collect_word_pieces(&pieces)
    }

    fn collect_word_pieces(&mut self, pieces: &[WordPieceWithSource]) -> Result<(), String> {
        for piece in pieces {
            match &piece.piece {
                WordPiece::CommandSubstitution(source)
                | WordPiece::BackquotedCommandSubstitution(source) => {
                    self.with_function_scope(true, |collector| collector.collect_source(source))?;
                }
                WordPiece::DoubleQuotedSequence(pieces)
                | WordPiece::GettextDoubleQuotedSequence(pieces) => {
                    self.collect_word_pieces(pieces)?;
                }
                WordPiece::ArithmeticExpression(expression) => {
                    self.collect_arithmetic_expression(expression)?;
                }
                WordPiece::ParameterExpansion(expression) => {
                    self.collect_parameter_expression(expression)?;
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn collect_parameter_expression(&mut self, expression: &ParameterExpr) -> Result<(), String> {
        match expression {
            ParameterExpr::Parameter { parameter, .. }
            | ParameterExpr::ParameterLength { parameter, .. }
            | ParameterExpr::Transform { parameter, .. }
            | ParameterExpr::UppercaseFirstChar { parameter, .. }
            | ParameterExpr::UppercasePattern { parameter, .. }
            | ParameterExpr::LowercaseFirstChar { parameter, .. }
            | ParameterExpr::LowercasePattern { parameter, .. }
            | ParameterExpr::RemoveSmallestSuffixPattern { parameter, .. }
            | ParameterExpr::RemoveLargestSuffixPattern { parameter, .. }
            | ParameterExpr::RemoveSmallestPrefixPattern { parameter, .. }
            | ParameterExpr::RemoveLargestPrefixPattern { parameter, .. }
            | ParameterExpr::UseDefaultValues { parameter, .. }
            | ParameterExpr::AssignDefaultValues { parameter, .. }
            | ParameterExpr::IndicateErrorIfNullOrUnset { parameter, .. }
            | ParameterExpr::UseAlternativeValue { parameter, .. }
            | ParameterExpr::Substring { parameter, .. }
            | ParameterExpr::ReplaceSubstring { parameter, .. } => {
                self.collect_parameter(parameter)?;
            }
            ParameterExpr::VariableNames { .. } | ParameterExpr::MemberKeys { .. } => {}
        }
        match expression {
            ParameterExpr::UseDefaultValues { default_value, .. }
            | ParameterExpr::AssignDefaultValues { default_value, .. } => {
                self.collect_optional_parameter_word(default_value.as_deref())?;
            }
            ParameterExpr::IndicateErrorIfNullOrUnset { error_message, .. } => {
                self.collect_optional_parameter_word(error_message.as_deref())?;
            }
            ParameterExpr::UseAlternativeValue {
                alternative_value, ..
            } => {
                self.collect_optional_parameter_word(alternative_value.as_deref())?;
            }
            ParameterExpr::RemoveSmallestSuffixPattern { pattern, .. }
            | ParameterExpr::RemoveLargestSuffixPattern { pattern, .. }
            | ParameterExpr::RemoveSmallestPrefixPattern { pattern, .. }
            | ParameterExpr::RemoveLargestPrefixPattern { pattern, .. }
            | ParameterExpr::UppercaseFirstChar { pattern, .. }
            | ParameterExpr::UppercasePattern { pattern, .. }
            | ParameterExpr::LowercaseFirstChar { pattern, .. }
            | ParameterExpr::LowercasePattern { pattern, .. } => {
                self.collect_optional_parameter_word(pattern.as_deref())?;
            }
            ParameterExpr::Substring { offset, length, .. } => {
                self.collect_arithmetic_expression(offset)?;
                if let Some(length) = length {
                    self.collect_arithmetic_expression(length)?;
                }
            }
            ParameterExpr::ReplaceSubstring {
                pattern,
                replacement,
                ..
            } => {
                self.collect_parameter_word(pattern)?;
                self.collect_optional_parameter_word(replacement.as_deref())?;
            }
            _ => {}
        }
        Ok(())
    }

    fn collect_parameter(&mut self, parameter: &Parameter) -> Result<(), String> {
        if let Parameter::NamedWithIndex { index, .. } = parameter {
            self.collect_parameter_word(index)?;
        }
        Ok(())
    }

    fn collect_optional_parameter_word(&mut self, value: Option<&str>) -> Result<(), String> {
        match value {
            Some(value) => self.collect_parameter_word(value),
            None => Ok(()),
        }
    }

    fn collect_parameter_word(&mut self, value: &str) -> Result<(), String> {
        let source = self.expand_positional_source(value);
        let pieces = brush_parser::word::parse(&source, &self.options)
            .map_err(|error| format!("Bash parameter word parse error: {error}"))?;
        self.collect_word_pieces(&pieces)
    }

    fn command_word(&self, word: &Word) -> Result<String, String> {
        let source = self.expand_positional_source(&word.value);
        let pieces = brush_parser::word::parse(&source, &self.options)
            .map_err(|error| format!("Bash word parse error: {error}"))?;
        Ok(static_word_pieces(&pieces).unwrap_or_else(|| DYNAMIC_SHELL_WORD.to_string()))
    }

    fn command_word_variants(&self, word: &Word) -> Result<Vec<String>, String> {
        let source = self.expand_positional_source(&word.value);
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

    pub(super) fn expand_positional_source(&self, source: &str) -> String {
        self.positional_context.as_ref().map_or_else(
            || source.to_string(),
            |context| {
                expand_shell_command(source, context.zero_argument.as_deref(), &context.arguments)
            },
        )
    }

    pub(super) fn expand_heredoc_source(&self, source: &str) -> String {
        self.positional_context.as_ref().map_or_else(
            || source.to_string(),
            |context| expand_heredoc(source, context.zero_argument.as_deref(), &context.arguments),
        )
    }

    fn with_positional_context<T>(
        &mut self,
        context: Option<PositionalContext>,
        collect: impl FnOnce(&mut Self) -> Result<T, String>,
    ) -> Result<T, String> {
        let saved = std::mem::replace(&mut self.positional_context, context);
        let result = collect(self);
        self.positional_context = saved;
        result
    }
}

fn static_token_measure(tokens: &[String]) -> usize {
    tokens
        .iter()
        .map(|token| token.len().saturating_add(1))
        .sum()
}

fn and_status(left: Option<bool>, right: Option<bool>) -> Option<bool> {
    match (left, right) {
        (Some(false), _) | (_, Some(false)) => Some(false),
        (Some(true), value) => value,
        (None, Some(true)) | (None, None) => None,
    }
}

fn or_status(left: Option<bool>, right: Option<bool>) -> Option<bool> {
    match (left, right) {
        (Some(true), _) | (_, Some(true)) => Some(true),
        (Some(false), value) => value,
        (None, Some(false)) | (None, None) => None,
    }
}
