use std::collections::{HashMap, HashSet};
use std::io::Cursor;

use brush_parser::ast::{
    AndOr, Command, CommandPrefixOrSuffixItem, CompoundCommand, CompoundList, ExtendedTestExpr,
    FunctionBody, IfClauseCommand, IoFileRedirectTarget, IoRedirect, Pipeline, Program,
    SimpleCommand, UnexpandedArithmeticExpr, Word,
};
use brush_parser::word::{Parameter, ParameterExpr, WordPiece, WordPieceWithSource};
use brush_parser::{Parser, ParserOptions};

mod static_execution;
mod static_words;
mod stdin_payload;
pub(super) mod unwrap;

use static_execution::{
    direct_command_name, static_env_split_tokens, static_eval_payload, static_exit_trap_payload,
    static_export_function_change, static_shell_command_payload, static_shell_is_bash,
    static_shell_reads_stdin, static_source_reads_stdin, static_unset_function_names,
};
use static_words::{
    append_word_variants, critical_brace_variants, expand_brace_pieces, static_word_pieces,
    StaticExpansionError,
};

const DYNAMIC_SHELL_WORD: &str = "__remem_dynamic_shell_word__";
const MAX_STATIC_WORD_VARIANTS: usize = 256;

pub(super) fn command_segments(source: &str) -> Result<Vec<Vec<String>>, String> {
    let mut collector = CommandCollector {
        options: ParserOptions::default(),
        segments: Vec::new(),
        functions: HashMap::new(),
        exported_functions: HashSet::new(),
        active_functions: HashSet::new(),
        execution_is_definite: true,
    };
    collector.collect_source(source)?;
    Ok(collector.segments)
}

struct CommandCollector {
    options: ParserOptions,
    segments: Vec<Vec<String>>,
    functions: HashMap<String, Vec<FunctionDefinition>>,
    exported_functions: HashSet<String>,
    active_functions: HashSet<String>,
    execution_is_definite: bool,
}

#[derive(Clone)]
struct FunctionDefinition {
    body: FunctionBody,
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
            self.collect_list(command)?;
        }
        Ok(())
    }

    fn collect_list(&mut self, list: &CompoundList) -> Result<(), String> {
        for item in &list.0 {
            self.collect_pipeline(&item.0.first)?;
            let mut static_success = self.static_pipeline_success(&item.0.first)?;
            for additional in &item.0.additional {
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
        for command in &pipeline.seq {
            self.collect_command(command)?;
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
            CompoundCommand::CaseClause(command) => {
                self.collect_word_commands(&command.value)?;
                for case in &command.cases {
                    for pattern in &case.patterns {
                        self.collect_word_commands(pattern)?;
                    }
                    if let Some(commands) = &case.cmd {
                        self.with_execution_certainty(false, |collector| {
                            collector.collect_list(commands)
                        })?;
                    }
                }
                Ok(())
            }
            CompoundCommand::IfClause(command) => self.collect_if_clause(command),
            CompoundCommand::WhileClause(command) | CompoundCommand::UntilClause(command) => {
                self.collect_list(&command.0)?;
                self.with_execution_certainty(false, |collector| {
                    collector.collect_list(&command.1.list)
                })
            }
            CompoundCommand::Coprocess(command) => {
                if let Some(name) = &command.name {
                    self.collect_word_commands(name)?;
                }
                self.with_function_scope(true, |collector| collector.collect_command(&command.body))
            }
        }
    }

    fn collect_if_clause(&mut self, command: &IfClauseCommand) -> Result<(), String> {
        self.collect_list(&command.condition)?;
        let status = self.static_list_success(&command.condition)?;
        if status != Some(false) {
            self.with_execution_certainty(status == Some(true), |collector| {
                collector.collect_list(&command.then)
            })?;
        }
        if status == Some(true) {
            return Ok(());
        }
        let mut branch_is_definite = status == Some(false);
        for branch in command.elses.iter().flatten() {
            let branch_status = if let Some(condition) = &branch.condition {
                self.with_execution_certainty(branch_is_definite, |collector| {
                    collector.collect_list(condition)
                })?;
                self.static_list_success(condition)?
            } else {
                Some(true)
            };
            if branch_status != Some(false) {
                self.with_execution_certainty(
                    branch_is_definite && branch_status == Some(true),
                    |collector| collector.collect_list(&branch.body),
                )?;
            }
            match branch_status {
                Some(true) => break,
                Some(false) => {}
                None => branch_is_definite = false,
            }
        }
        Ok(())
    }

    fn static_list_success(&self, list: &CompoundList) -> Result<Option<bool>, String> {
        let Some(item) = list.0.last() else {
            return Ok(None);
        };
        let mut status = self.static_pipeline_success(&item.0.first)?;
        for additional in &item.0.additional {
            status = match additional {
                AndOr::And(pipeline) => and_status(status, self.static_pipeline_success(pipeline)?),
                AndOr::Or(pipeline) => or_status(status, self.static_pipeline_success(pipeline)?),
            };
        }
        Ok(status)
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
        self.collect_static_function_call(&tokens)?;
        if let Some(payload) = static_exit_trap_payload(&tokens) {
            self.with_function_scope(true, |collector| collector.collect_source(payload))?;
        }
        if let Some(payload) = static_eval_payload(&tokens) {
            self.collect_source(&payload)?;
        }
        if let Some(payload) = static_shell_command_payload(&tokens) {
            self.with_child_shell_scope(static_shell_is_bash(&tokens), |collector| {
                collector.collect_source(payload)
            })?;
        }
        if static_shell_reads_stdin(&tokens) {
            if let Some(payload) = self.effective_stdin_payload(command)? {
                self.with_child_shell_scope(static_shell_is_bash(&tokens), |collector| {
                    collector.collect_source(&payload)
                })?;
            }
        } else if static_source_reads_stdin(&tokens) {
            if let Some(payload) = self.effective_stdin_payload(command)? {
                self.collect_source(&payload)?;
            }
        }
        self.segments.push(tokens);
        Ok(())
    }

    fn collect_static_function_call(&mut self, tokens: &[String]) -> Result<(), String> {
        let Some(name) = direct_command_name(tokens) else {
            return Ok(());
        };
        let Some(definitions) = self.functions.get(name).cloned() else {
            return Ok(());
        };
        if !self.active_functions.insert(name.to_string()) {
            return Ok(());
        }
        let result = (|| {
            for definition in definitions {
                self.with_execution_certainty(definition.is_definite, |collector| {
                    collector.collect_compound_command(&definition.body.0)?;
                    if let Some(redirects) = &definition.body.1 {
                        for redirect in &redirects.0 {
                            collector.collect_redirect_commands(redirect)?;
                        }
                    }
                    Ok(())
                })?;
            }
            Ok(())
        })();
        self.active_functions.remove(name);
        result
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
        let pieces = brush_parser::word::parse(&expression.value, &self.options)
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
        let pieces = brush_parser::word::parse(value, &self.options)
            .map_err(|error| format!("Bash parameter word parse error: {error}"))?;
        self.collect_word_pieces(&pieces)
    }

    fn with_function_scope<T>(
        &mut self,
        inherit: bool,
        collect: impl FnOnce(&mut Self) -> Result<T, String>,
    ) -> Result<T, String> {
        let saved_functions = self.functions.clone();
        let saved_exported_functions = self.exported_functions.clone();
        let saved_active_functions = self.active_functions.clone();
        if !inherit {
            self.functions.clear();
            self.exported_functions.clear();
            self.active_functions.clear();
        }
        let result = collect(self);
        self.functions = saved_functions;
        self.exported_functions = saved_exported_functions;
        self.active_functions = saved_active_functions;
        result
    }

    fn with_child_shell_scope<T>(
        &mut self,
        inherit_exported: bool,
        collect: impl FnOnce(&mut Self) -> Result<T, String>,
    ) -> Result<T, String> {
        let saved_functions = self.functions.clone();
        let saved_exports = self.exported_functions.clone();
        let saved_active = self.active_functions.clone();
        if inherit_exported {
            self.functions
                .retain(|name, _| saved_exports.contains(name));
            self.exported_functions
                .retain(|name| self.functions.contains_key(name));
        } else {
            self.functions.clear();
            self.exported_functions.clear();
        }
        self.active_functions.clear();
        let result = collect(self);
        self.functions = saved_functions;
        self.exported_functions = saved_exports;
        self.active_functions = saved_active;
        result
    }

    fn with_execution_certainty<T>(
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

    fn command_word(&self, word: &Word) -> Result<String, String> {
        let pieces = brush_parser::word::parse(&word.value, &self.options)
            .map_err(|error| format!("Bash word parse error: {error}"))?;
        Ok(static_word_pieces(&pieces).unwrap_or_else(|| DYNAMIC_SHELL_WORD.to_string()))
    }

    fn command_word_variants(&self, word: &Word) -> Result<Vec<String>, String> {
        let Some(brace_pieces) =
            brush_parser::word::parse_brace_expansions(&word.value, &self.options)
                .map_err(|error| format!("Bash brace expansion parse error: {error}"))?
        else {
            return Ok(vec![self.command_word(word)?]);
        };
        let expanded = match expand_brace_pieces(&brace_pieces) {
            Ok(expanded) => expanded,
            Err(StaticExpansionError::Limit) => {
                let mut variants = critical_brace_variants(&brace_pieces);
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
