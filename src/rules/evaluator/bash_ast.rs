use std::io::Cursor;

use brush_parser::ast::{
    AndOr, Command, CommandPrefixOrSuffixItem, CompoundCommand, CompoundList, ExtendedTestExpr,
    IoFileRedirectTarget, IoRedirect, Pipeline, Program, SimpleCommand, UnexpandedArithmeticExpr,
    Word,
};
use brush_parser::word::{
    BraceExpressionMember, BraceExpressionOrText, WordPiece, WordPieceWithSource,
};
use brush_parser::{Parser, ParserOptions};

const DYNAMIC_SHELL_WORD: &str = "__remem_dynamic_shell_word__";
const MAX_STATIC_WORD_VARIANTS: usize = 256;

pub(super) fn command_segments(source: &str) -> Result<Vec<Vec<String>>, String> {
    let mut collector = CommandCollector {
        options: ParserOptions::default(),
        segments: Vec::new(),
    };
    collector.collect_source(source)?;
    Ok(collector.segments)
}

struct CommandCollector {
    options: ParserOptions,
    segments: Vec<Vec<String>>,
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
            for additional in &item.0.additional {
                match additional {
                    AndOr::And(pipeline) | AndOr::Or(pipeline) => {
                        self.collect_pipeline(pipeline)?;
                    }
                }
            }
        }
        Ok(())
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
            Command::Function(_) => Ok(()),
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
                self.collect_list(&command.body.list)
            }
            CompoundCommand::BraceGroup(command) => self.collect_list(&command.list),
            CompoundCommand::Subshell(command) => self.collect_list(&command.list),
            CompoundCommand::ForClause(command) => {
                if let Some(values) = &command.values {
                    for value in values {
                        self.collect_word_commands(value)?;
                    }
                }
                self.collect_list(&command.body.list)
            }
            CompoundCommand::CaseClause(command) => {
                self.collect_word_commands(&command.value)?;
                for case in &command.cases {
                    for pattern in &case.patterns {
                        self.collect_word_commands(pattern)?;
                    }
                    if let Some(commands) = &case.cmd {
                        self.collect_list(commands)?;
                    }
                }
                Ok(())
            }
            CompoundCommand::IfClause(command) => {
                self.collect_list(&command.condition)?;
                self.collect_list(&command.then)?;
                if let Some(elses) = &command.elses {
                    for branch in elses {
                        if let Some(condition) = &branch.condition {
                            self.collect_list(condition)?;
                        }
                        self.collect_list(&branch.body)?;
                    }
                }
                Ok(())
            }
            CompoundCommand::WhileClause(command) | CompoundCommand::UntilClause(command) => {
                self.collect_list(&command.0)?;
                self.collect_list(&command.1.list)
            }
            CompoundCommand::Coprocess(command) => {
                if let Some(name) = &command.name {
                    self.collect_word_commands(name)?;
                }
                self.collect_command(&command.body)
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
                    CommandPrefixOrSuffixItem::Word(word) => {
                        self.collect_word_commands(word)?;
                        append_word_variants(&mut segments, self.command_word_variants(word)?);
                    }
                    _ => self.collect_command_item(item)?,
                }
            }
        }
        for tokens in segments {
            if let Some(payload) = static_shell_command_payload(&tokens) {
                self.collect_source(payload)?;
            }
            self.segments.push(tokens);
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
                self.collect_list(&command.list)
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
                IoFileRedirectTarget::ProcessSubstitution(_, command) => {
                    self.collect_list(&command.list)
                }
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
                    self.collect_source(source)?;
                }
                WordPiece::DoubleQuotedSequence(pieces)
                | WordPiece::GettextDoubleQuotedSequence(pieces) => {
                    self.collect_word_pieces(pieces)?;
                }
                WordPiece::ArithmeticExpression(expression) => {
                    self.collect_arithmetic_expression(expression)?;
                }
                _ => {}
            }
        }
        Ok(())
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
                return Ok(vec![DYNAMIC_SHELL_WORD.to_string()]);
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

fn append_word_variants(segments: &mut Vec<Vec<String>>, variants: Vec<String>) {
    if variants.is_empty()
        || segments.len().saturating_mul(variants.len()) > MAX_STATIC_WORD_VARIANTS
    {
        for segment in segments {
            segment.push(DYNAMIC_SHELL_WORD.to_string());
        }
        return;
    }
    let prefixes = std::mem::take(segments);
    for prefix in prefixes {
        for variant in &variants {
            let mut expanded = prefix.clone();
            expanded.push(variant.clone());
            segments.push(expanded);
        }
    }
}

enum StaticExpansionError {
    Limit,
    Invalid(String),
}

fn static_shell_command_payload(tokens: &[String]) -> Option<&str> {
    let mut command_index = tokens
        .iter()
        .position(|token| !super::is_env_assignment(token))?;
    loop {
        command_index = match tokens.get(command_index)?.as_str() {
            "command" => super::command_wrapper_target(tokens, command_index)?,
            "env" => super::env_wrapper_target(tokens, command_index)?,
            _ => break,
        };
    }
    let shell = std::path::Path::new(tokens.get(command_index)?)
        .file_name()
        .and_then(|value| value.to_str())?;
    if !matches!(shell, "bash" | "dash" | "ksh" | "sh" | "zsh") {
        return None;
    }
    let mut index = command_index + 1;
    while let Some(option) = tokens.get(index) {
        if option == "--" || !option.starts_with('-') || option == "-" {
            return None;
        }
        let carries_command = option == "-c"
            || option
                .strip_prefix('-')
                .is_some_and(|flags| !flags.starts_with('-') && flags.contains('c'));
        if carries_command {
            let payload = tokens.get(index + 1)?;
            return (payload != DYNAMIC_SHELL_WORD).then_some(payload.as_str());
        }
        index += 1;
    }
    None
}

fn expand_brace_pieces(
    pieces: &[BraceExpressionOrText],
) -> Result<Vec<String>, StaticExpansionError> {
    let mut variants = vec![String::new()];
    for piece in pieces {
        let suffixes = match piece {
            BraceExpressionOrText::Text(text) => vec![text.clone()],
            BraceExpressionOrText::Expr(expression) => expand_brace_expression(expression)?,
        };
        append_text_variants(&mut variants, &suffixes)?;
    }
    Ok(variants)
}

fn expand_brace_expression(
    expression: &[BraceExpressionMember],
) -> Result<Vec<String>, StaticExpansionError> {
    let mut variants = Vec::new();
    for member in expression {
        match member {
            BraceExpressionMember::Child(pieces) => variants.extend(expand_brace_pieces(pieces)?),
            BraceExpressionMember::NumberSequence {
                start,
                end,
                increment,
            } => {
                let values = inclusive_i64_sequence(*start, *end, *increment)?;
                variants.extend(values.into_iter().map(|value| value.to_string()));
            }
            BraceExpressionMember::CharSequence {
                start,
                end,
                increment,
            } => {
                let values = inclusive_i64_sequence(*start as i64, *end as i64, *increment)?;
                for value in values {
                    let value = u32::try_from(value)
                        .ok()
                        .and_then(char::from_u32)
                        .ok_or_else(|| {
                            StaticExpansionError::Invalid(
                                "Bash brace expansion produced an invalid character".to_string(),
                            )
                        })?;
                    variants.push(value.to_string());
                }
            }
        }
        if variants.len() > MAX_STATIC_WORD_VARIANTS {
            return Err(StaticExpansionError::Limit);
        }
    }
    Ok(variants)
}

fn inclusive_i64_sequence(
    start: i64,
    end: i64,
    increment: i64,
) -> Result<Vec<i64>, StaticExpansionError> {
    if increment == 0 || (start < end && increment < 0) || (start > end && increment > 0) {
        return Err(StaticExpansionError::Invalid(
            "Bash brace expansion has an invalid sequence increment".to_string(),
        ));
    }
    let mut values = Vec::new();
    let mut value = start;
    while if increment > 0 {
        value <= end
    } else {
        value >= end
    } {
        if values.len() == MAX_STATIC_WORD_VARIANTS {
            return Err(StaticExpansionError::Limit);
        }
        values.push(value);
        let Some(next) = value.checked_add(increment) else {
            break;
        };
        value = next;
    }
    Ok(values)
}

fn append_text_variants(
    variants: &mut Vec<String>,
    suffixes: &[String],
) -> Result<(), StaticExpansionError> {
    if suffixes.is_empty()
        || variants.len().saturating_mul(suffixes.len()) > MAX_STATIC_WORD_VARIANTS
    {
        return Err(StaticExpansionError::Limit);
    }
    let prefixes = std::mem::take(variants);
    for prefix in prefixes {
        for suffix in suffixes {
            let mut expanded = prefix.clone();
            expanded.push_str(suffix);
            variants.push(expanded);
        }
    }
    Ok(())
}

fn static_word_pieces(pieces: &[WordPieceWithSource]) -> Option<String> {
    let mut value = String::new();
    for piece in pieces {
        match &piece.piece {
            WordPiece::Text(text) | WordPiece::SingleQuotedText(text) => value.push_str(text),
            WordPiece::EscapeSequence(text) => {
                let escaped = text.strip_prefix('\\')?;
                if escaped != "\n" {
                    value.push_str(escaped);
                }
            }
            WordPiece::DoubleQuotedSequence(pieces)
            | WordPiece::GettextDoubleQuotedSequence(pieces) => {
                value.push_str(&static_word_pieces(pieces)?);
            }
            WordPiece::AnsiCQuotedText(text) => {
                value.push_str(&decode_ansi_c_quoted_text(text)?);
            }
            WordPiece::TildeExpansion(_)
            | WordPiece::ParameterExpansion(_)
            | WordPiece::CommandSubstitution(_)
            | WordPiece::BackquotedCommandSubstitution(_)
            | WordPiece::ArithmeticExpression(_) => return None,
        }
    }
    Some(value)
}

fn decode_ansi_c_quoted_text(text: &str) -> Option<String> {
    let mut bytes = Vec::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            push_char_bytes(&mut bytes, ch);
            continue;
        }
        let escaped = chars.next()?;
        match escaped {
            'a' => bytes.push(0x07),
            'b' => bytes.push(0x08),
            'e' | 'E' => bytes.push(0x1b),
            'f' => bytes.push(0x0c),
            'n' => bytes.push(b'\n'),
            'r' => bytes.push(b'\r'),
            't' => bytes.push(b'\t'),
            'v' => bytes.push(0x0b),
            '\\' => bytes.push(b'\\'),
            '\'' => bytes.push(b'\''),
            'c' => {
                let control = chars.next()?;
                if !control.is_ascii() {
                    return None;
                }
                let control = control.to_ascii_uppercase() as u8;
                bytes.push(if control == b'?' {
                    0x7f
                } else {
                    control & 0x1f
                });
            }
            'x' => bytes.push(take_digits(&mut chars, 16, 2)? as u8),
            'u' => {
                let decoded = char::from_u32(take_digits(&mut chars, 16, 4)?)?;
                push_char_bytes(&mut bytes, decoded);
            }
            'U' => {
                let decoded = char::from_u32(take_digits(&mut chars, 16, 8)?)?;
                push_char_bytes(&mut bytes, decoded);
            }
            '0' => bytes.push(take_digits(&mut chars, 8, 3).unwrap_or(0) as u8),
            '1'..='7' => {
                let mut value = escaped.to_digit(8)?;
                for _ in 0..2 {
                    let Some(digit) = chars.peek().and_then(|ch| ch.to_digit(8)) else {
                        break;
                    };
                    chars.next();
                    value = value * 8 + digit;
                }
                bytes.push(value as u8);
            }
            _ => {
                bytes.push(b'\\');
                push_char_bytes(&mut bytes, escaped);
            }
        }
    }
    if bytes.contains(&0) {
        return None;
    }
    String::from_utf8(bytes).ok()
}

fn take_digits<I>(chars: &mut std::iter::Peekable<I>, radix: u32, max: usize) -> Option<u32>
where
    I: Iterator<Item = char>,
{
    let mut value = 0;
    let mut count = 0;
    while count < max {
        let Some(digit) = chars.peek().and_then(|ch| ch.to_digit(radix)) else {
            break;
        };
        chars.next();
        value = value * radix + digit;
        count += 1;
    }
    (count > 0).then_some(value)
}

fn push_char_bytes(bytes: &mut Vec<u8>, ch: char) {
    let mut encoded = [0; 4];
    bytes.extend_from_slice(ch.encode_utf8(&mut encoded).as_bytes());
}
