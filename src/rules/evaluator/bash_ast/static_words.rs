use brush_parser::word::{
    BraceExpressionMember, BraceExpressionOrText, WordPiece, WordPieceWithSource,
};

use super::{DYNAMIC_SHELL_WORD, MAX_STATIC_WORD_VARIANTS};
use crate::rules::evaluator::git_push_arg_changes_force_state;

const CRITICAL_STATIC_TOKENS: &[&str] = &[
    "git",
    "push",
    "commit",
    "--force",
    "-f",
    "--mirror",
    "--trailer",
    "command",
    "env",
    "exec",
    "eval",
    "bash",
    "dash",
    "ksh",
    "sh",
    "zsh",
];

pub(super) fn append_word_variants(segments: &mut [Vec<String>], variants: Vec<String>) {
    for segment in segments {
        if variants.is_empty() {
            segment.push(DYNAMIC_SHELL_WORD.to_string());
        } else {
            segment.extend(variants.iter().cloned());
        }
    }
}

pub(super) fn critical_brace_variants(pieces: &[BraceExpressionOrText]) -> Vec<String> {
    let mut variants = security_brace_variants(pieces)
        .into_iter()
        .filter(|value| is_critical_static_token(value))
        .collect::<Vec<_>>();
    let mut seen = std::collections::HashSet::new();
    variants.reverse();
    variants.retain(|variant| seen.insert(variant.clone()));
    variants.reverse();
    variants
}

fn is_critical_static_token(value: &str) -> bool {
    CRITICAL_STATIC_TOKENS.contains(&value)
        || git_push_arg_changes_force_state(value)
        || value.starts_with("--trailer=")
        || value.starts_with('+') && value.len() > 1
}

fn security_brace_variants(pieces: &[BraceExpressionOrText]) -> Vec<String> {
    let mut variants = vec![String::new()];
    for piece in pieces {
        let suffixes = match piece {
            BraceExpressionOrText::Text(text) => vec![text.clone()],
            BraceExpressionOrText::Expr(expression) => {
                let mut suffixes = Vec::new();
                for member in expression {
                    match member {
                        BraceExpressionMember::Child(child) => {
                            suffixes.extend(security_brace_variants(child));
                        }
                        BraceExpressionMember::NumberSequence { start, end, .. } => {
                            suffixes.push(start.to_string());
                            suffixes.push(end.to_string());
                        }
                        BraceExpressionMember::CharSequence {
                            start,
                            end,
                            increment,
                        } => {
                            suffixes.push(start.to_string());
                            suffixes.push(end.to_string());
                            for candidate in ['-', '+', ':', 'f', 'm', 'i', 'o'] {
                                if sequence_contains(
                                    *start as i64,
                                    *end as i64,
                                    *increment,
                                    candidate as i64,
                                ) {
                                    suffixes.push(candidate.to_string());
                                }
                            }
                        }
                    }
                    summarize_security_variants(&mut suffixes);
                }
                suffixes
            }
        };
        let prefixes = std::mem::take(&mut variants);
        for prefix in prefixes {
            for suffix in &suffixes {
                variants.push(format!("{prefix}{suffix}"));
            }
        }
        summarize_security_variants(&mut variants);
    }
    variants
}

fn summarize_security_variants(variants: &mut Vec<String>) {
    let mut seen = std::collections::HashSet::new();
    variants.reverse();
    variants.retain(|variant| seen.insert(variant.clone()));
    variants.reverse();
    if variants.len() <= MAX_STATIC_WORD_VARIANTS {
        return;
    }
    let mut retained = (0..variants.len()).collect::<Vec<_>>();
    retained.sort_unstable_by(|left, right| {
        security_variant_score(&variants[*right])
            .cmp(&security_variant_score(&variants[*left]))
            .then_with(|| right.cmp(left))
    });
    retained.truncate(MAX_STATIC_WORD_VARIANTS);
    retained.sort_unstable();
    *variants = retained
        .into_iter()
        .map(|index| variants[index].clone())
        .collect();
}

fn security_variant_score(value: &str) -> usize {
    if is_critical_static_token(value) {
        return 100;
    }
    let before_force = value.split_once('f').map_or(value, |(prefix, _)| prefix);
    let bare = value.trim_start_matches('-');
    if value.contains('f') && !before_force.contains('o') {
        return 50;
    }
    if !bare.is_empty() && ("force".ends_with(bare) || "mirror".ends_with(bare)) {
        return 50;
    }
    if value.starts_with('+') || "mirror".starts_with(bare) {
        return 40;
    }
    usize::from(value.chars().any(|ch| matches!(ch, '-' | '+' | 'f' | 'm')))
}

fn sequence_contains(start: i64, end: i64, increment: i64, value: i64) -> bool {
    if increment == 0
        || value < start.min(end)
        || value > start.max(end)
        || start < end && increment < 0
        || start > end && increment > 0
    {
        return false;
    }
    value
        .checked_sub(start)
        .is_some_and(|distance| distance % increment == 0)
}

pub(super) enum StaticExpansionError {
    Limit,
    Invalid(String),
}

pub(super) fn expand_brace_pieces(
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
        || suffixes.len() > MAX_STATIC_WORD_VARIANTS
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

pub(super) fn static_word_pieces(pieces: &[WordPieceWithSource]) -> Option<String> {
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
