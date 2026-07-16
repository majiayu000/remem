use std::io::Cursor;

use brush_parser::ast::{
    AndOr, Command, CommandPrefixOrSuffixItem, CompoundCommand, CompoundList, ExtendedTestExpr,
    IoFileRedirectTarget, IoRedirect, Pipeline, Program, SimpleCommand, UnexpandedArithmeticExpr,
    Word,
};
use brush_parser::word::{WordPiece, WordPieceWithSource};
use brush_parser::{Parser, ParserOptions};

const DYNAMIC_SHELL_WORD: &str = "__remem_dynamic_shell_word__";

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
        let mut tokens = Vec::new();
        if let Some(prefix) = &command.prefix {
            for item in &prefix.0 {
                match item {
                    CommandPrefixOrSuffixItem::AssignmentWord(_, word) => {
                        tokens.push(self.command_word(word)?);
                    }
                    _ => self.collect_command_item(item)?,
                }
            }
        }
        self.collect_word_commands(name)?;
        tokens.push(self.command_word(name)?);
        if let Some(suffix) = &command.suffix {
            for item in &suffix.0 {
                match item {
                    CommandPrefixOrSuffixItem::Word(word) => {
                        self.collect_word_commands(word)?;
                        tokens.push(self.command_word(word)?);
                    }
                    _ => self.collect_command_item(item)?,
                }
            }
        }
        self.segments.push(tokens);
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
            IoRedirect::HereDocument(_, _) => Ok(()),
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
