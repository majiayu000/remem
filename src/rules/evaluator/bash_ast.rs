use std::collections::{HashMap, HashSet};
use std::io::Cursor;

use brush_parser::ast::{
    Command, CommandPrefixOrSuffixItem, CompoundCommand, ExtendedTestExpr, FunctionBody,
    IoFileRedirectTarget, IoRedirect, Pipeline, Program, SimpleCommand, UnexpandedArithmeticExpr,
    Word,
};
use brush_parser::word::{Parameter, ParameterExpr, WordPiece, WordPieceWithSource};
use brush_parser::{Parser, ParserOptions};

mod alternative_state;
mod command_resolution;
mod control_flow;
mod function_args;
mod shell_state;
mod static_execution;
mod static_words;
mod stdin_payload;
pub(super) mod unwrap;

use function_args::{
    bare_shell_positional_variant_fields, expand_shell_arithmetic, expand_shell_command,
    has_shell_positional_reference,
};
use static_words::{static_source_word_variants, static_word_pieces};

#[cfg(test)]
pub(super) use shell_state::bound_possible_positional_arguments;

const DYNAMIC_SHELL_WORD: &str = "__remem_dynamic_shell_word__";
pub(super) const MAX_STATIC_WORD_VARIANTS: usize = 256;

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
        positional_execution_is_definite: true,
        last_positional_status: None,
        last_positional_success: None,
        last_positional_failure: None,
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
    positional_execution_is_definite: bool,
    last_positional_status: Option<bool>,
    last_positional_success: Option<PositionalContext>,
    last_positional_failure: Option<PositionalContext>,
    inherited_stdin: Option<String>,
    positional_context: Option<PositionalContext>,
}

#[derive(Clone)]
struct PositionalContext {
    zero_argument: Option<String>,
    arguments: Vec<String>,
    possible_arguments: Vec<Vec<String>>,
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
    is_expandable: bool,
    is_definitely_expandable: bool,
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

    fn static_pipeline_success(&self, pipeline: &Pipeline) -> Result<Option<bool>, String> {
        let [Command::Simple(command)] = pipeline.seq.as_slice() else {
            return Ok(None);
        };
        if let Some(success) = self.last_positional_status {
            return Ok(Some(if pipeline.bang { !success } else { success }));
        }
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
        self.last_positional_status = None;
        self.last_positional_success = None;
        self.last_positional_failure = None;
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
                let name = self.command_word_without_positional(&function.fname)?;
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
                        if !definitions.iter().any(|definition| definition.is_definite) {
                            for definition in definitions.iter_mut() {
                                definition.is_definite = false;
                            }
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
        let mut expanded_command_start = None;
        if let Some(prefix) = &command.prefix {
            for item in &prefix.0 {
                match item {
                    CommandPrefixOrSuffixItem::AssignmentWord(assignment, word) => {
                        if expanded_command_start.is_none()
                            && self.positional_context.is_some()
                            && has_shell_positional_reference(&assignment.name.to_string())
                        {
                            expanded_command_start = Some(segments.first().map_or(0, Vec::len));
                        }
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
        let command_start = segments.first().map_or(0, Vec::len);
        if expanded_command_start.is_none()
            && self.positional_context.is_some()
            && has_shell_positional_reference(&name.value)
        {
            expanded_command_start = Some(command_start);
        }
        self.collect_word_commands(name)?;
        self.append_command_name_variants(&mut segments, name)?;
        if let Some(suffix) = &command.suffix {
            for item in &suffix.0 {
                match item {
                    CommandPrefixOrSuffixItem::Word(word)
                    | CommandPrefixOrSuffixItem::AssignmentWord(_, word) => {
                        self.collect_word_commands(word)?;
                        self.append_command_argument_variants(&mut segments, word)?;
                    }
                    _ => self.collect_command_item(item)?,
                }
            }
        }
        if let Some(command_start) = expanded_command_start {
            for segment in &mut segments {
                if let Some(command_word) = segment.get_mut(command_start) {
                    unwrap::mark_expanded_command_word(command_word);
                }
            }
        }
        if !command_resolution::command_has_fallible_setup(command)
            && self.collect_correlated_positional_tokens(&segments)
        {
            return Ok(());
        }
        if self.collect_correlated_command_variants(&segments, command)? {
            return Ok(());
        }
        let mut seen = HashSet::new();
        segments.retain(|tokens| seen.insert(tokens.clone()));
        for tokens in segments {
            self.collect_static_tokens(tokens, command)?;
        }
        Ok(())
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
        let pieces = brush_parser::word::parse(&word.value, &self.options)
            .map_err(|error| format!("Bash word parse error: {error}"))?;
        self.collect_word_pieces(&pieces)
    }

    fn collect_arithmetic_expression(
        &mut self,
        expression: &UnexpandedArithmeticExpr,
    ) -> Result<(), String> {
        let source = self.expand_arithmetic_source(&expression.value)?;
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
        let source = self.expand_positional_source(value)?;
        let pieces = brush_parser::word::parse(&source, &self.options)
            .map_err(|error| format!("Bash parameter word parse error: {error}"))?;
        self.collect_word_pieces(&pieces)
    }

    fn command_word(&self, word: &Word) -> Result<String, String> {
        let source = self.expand_positional_source(&word.value)?;
        let pieces = brush_parser::word::parse(&source, &self.options)
            .map_err(|error| format!("Bash word parse error: {error}"))?;
        Ok(static_word_pieces(&pieces).unwrap_or_else(|| DYNAMIC_SHELL_WORD.to_string()))
    }

    fn command_word_without_positional(&self, word: &Word) -> Result<String, String> {
        let pieces = brush_parser::word::parse(&word.value, &self.options)
            .map_err(|error| format!("Bash word parse error: {error}"))?;
        Ok(static_word_pieces(&pieces).unwrap_or_else(|| DYNAMIC_SHELL_WORD.to_string()))
    }

    fn command_word_variants(&self, word: &Word) -> Result<Vec<String>, String> {
        if let Some(context) = &self.positional_context {
            if let Some(fields) = bare_shell_positional_variant_fields(
                &word.value,
                &self.options,
                context.zero_argument.as_deref(),
                &context.arguments,
                &context.possible_arguments,
            )? {
                return Ok(fields);
            }
            let mut variants = Vec::new();
            for arguments in std::iter::once(context.arguments.as_slice())
                .chain(context.possible_arguments.iter().map(Vec::as_slice))
            {
                let source = expand_shell_command(
                    &word.value,
                    &self.options,
                    context.zero_argument.as_deref(),
                    arguments,
                )?;
                variants.extend(static_source_word_variants(&source, &self.options)?);
            }
            return Ok(variants);
        }
        static_source_word_variants(&word.value, &self.options)
    }
    pub(super) fn expand_positional_source(&self, source: &str) -> Result<String, String> {
        self.positional_context.as_ref().map_or_else(
            || Ok(source.to_string()),
            |context| {
                expand_shell_command(
                    source,
                    &self.options,
                    context.zero_argument.as_deref(),
                    &context.arguments,
                )
            },
        )
    }

    fn expand_arithmetic_source(&self, source: &str) -> Result<String, String> {
        self.positional_context.as_ref().map_or_else(
            || Ok(source.to_string()),
            |context| {
                expand_shell_arithmetic(
                    source,
                    &self.options,
                    context.zero_argument.as_deref(),
                    &context.arguments,
                )
            },
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
