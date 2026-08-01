use std::ops::Range;

use super::{is_cn_number_component, leading_ascii_digits};

const CJK_TEMPORAL_INTRODUCERS: &[&str] = &[
    "截至", "截止", "自从", "早在", "直到", "在", "于", "自", "从", "至", "到",
];

pub(super) fn has_phrase_context(query: &str, span: &Range<usize>) -> bool {
    phrase_left_boundary_is_valid(query, span.start, false)
        && phrase_right_boundary_is_valid(query, span.end, false)
}

pub(super) fn has_cjk_phrase_context(query: &str, span: &Range<usize>) -> bool {
    phrase_left_boundary_is_valid(query, span.start, true)
        && phrase_right_boundary_is_valid(query, span.end, true)
}

pub(super) fn cjk_day_phrase_span(query: &str, span: &Range<usize>) -> Option<Range<usize>> {
    if !phrase_left_boundary_is_valid(query, span.start, true) {
        return None;
    }
    if cjk_clock_time_suffix_start(query, span.end).is_some() {
        let suffix_end = cjk_clock_time_suffix_end(query, span.end)?;
        return Some(span.start..suffix_end);
    }
    if phrase_right_boundary_is_valid(query, span.end, true) {
        return Some(span.clone());
    }
    None
}

pub(super) fn has_recent_phrase_context(query: &str, span: &Range<usize>) -> bool {
    phrase_left_boundary_is_valid(query, span.start, true)
        && query[span.end..].chars().next().is_none_or(|character| {
            character.is_ascii_digit() || phrase_right_boundary_is_valid(query, span.end, true)
        })
}

pub(super) fn date_span_with_context(query: &str, span: &Range<usize>) -> Option<Range<usize>> {
    let left = &query[..span.start];
    let left_is_natural = match left.chars().next_back() {
        None => true,
        Some(character) if character.is_whitespace() => true,
        Some(character) if is_structural_separator(character) => false,
        Some(character) if character.is_ascii_alphanumeric() || character.is_numeric() => false,
        Some(character) if character.is_alphanumeric() => CJK_TEMPORAL_INTRODUCERS
            .iter()
            .any(|introducer| left.ends_with(introducer)),
        Some(_) => true,
    };
    if !left_is_natural {
        return None;
    }
    if cjk_clock_time_suffix_start(query, span.end).is_some() {
        let suffix_end = cjk_clock_time_suffix_end(query, span.end)?;
        return Some(span.start..suffix_end);
    }
    if phrase_right_boundary_is_valid(query, span.end, true) {
        return Some(span.clone());
    }
    None
}

pub(super) fn span_with_cjk_temporal_introducer(query: &str, span: &Range<usize>) -> Range<usize> {
    let left = query[..span.start].trim_end_matches(char::is_whitespace);
    for introducer in CJK_TEMPORAL_INTRODUCERS {
        if left.ends_with(introducer) {
            return left.len() - introducer.len()..span.end;
        }
    }
    span.clone()
}

fn cjk_clock_time_suffix_end(query: &str, start: usize) -> Option<usize> {
    let start = cjk_clock_time_suffix_start(query, start)?;
    let hour_text = leading_ascii_digits(&query[start..])?;
    let hour = hour_text.parse::<u32>().ok()?;
    if hour > 23 {
        return None;
    }

    let hour_end = start + hour_text.len();
    let marker = query[hour_end..].chars().next()?;
    if !matches!(marker, '点' | '时' | ':' | '：') {
        return None;
    }
    let mut suffix_end = hour_end + marker.len_utf8();
    let remainder = &query[suffix_end..];

    if matches!(marker, ':' | '：') {
        let minute_text = leading_ascii_digits(remainder)?;
        if minute_text.len() != 2 {
            return None;
        }
        let minute = minute_text.parse::<u32>().ok()?;
        if minute > 59 {
            return None;
        }
        suffix_end += minute_text.len();
    } else {
        if let Some(half) = remainder
            .chars()
            .next()
            .filter(|character| *character == '半')
        {
            suffix_end += half.len_utf8();
        } else if remainder
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_digit())
        {
            let minute_text = leading_ascii_digits(remainder)?;
            let minute = minute_text.parse::<u32>().ok()?;
            if minute > 59 {
                return None;
            }
            let minute_end = suffix_end + minute_text.len();
            let minute_marker = query[minute_end..].chars().next()?;
            if minute_marker != '分' {
                return None;
            }
            suffix_end = minute_end + minute_marker.len_utf8();
        }

        if query[suffix_end..]
            .chars()
            .next()
            .is_some_and(|character| matches!(character, '钟' | '整'))
        {
            suffix_end += query[suffix_end..].chars().next()?.len_utf8();
        }
    }

    phrase_right_boundary_is_valid(query, suffix_end, true).then_some(suffix_end)
}

fn cjk_clock_time_suffix_start(query: &str, start: usize) -> Option<usize> {
    let remainder = &query[start..];
    let trimmed = remainder.trim_start_matches(char::is_whitespace);
    let clock_start = start + remainder.len() - trimmed.len();
    let numeric_end = trimmed
        .char_indices()
        .take_while(|(_, character)| character.is_numeric() || is_cn_number_component(*character))
        .last()
        .map(|(index, character)| index + character.len_utf8())?;
    trimmed[numeric_end..]
        .chars()
        .next()
        .filter(|marker| matches!(marker, '点' | '时' | ':' | '：'))
        .map(|_| clock_start)
}

fn phrase_left_boundary_is_valid(query: &str, start: usize, allow_cjk: bool) -> bool {
    let Some((_, character)) = query[..start].char_indices().next_back() else {
        return true;
    };
    if is_structural_separator(character) {
        return false;
    }
    phrase_neighbor_is_valid(character, allow_cjk)
}

fn phrase_right_boundary_is_valid(query: &str, end: usize, allow_cjk: bool) -> bool {
    let Some(character) = query[end..].chars().next() else {
        return true;
    };
    if is_structural_separator(character) {
        let run_end = query[end..]
            .char_indices()
            .take_while(|(_, character)| is_structural_separator(*character))
            .last()
            .map_or(end, |(index, character)| end + index + character.len_utf8());
        return query[end..run_end]
            .chars()
            .all(is_sentence_boundary_separator)
            && query[run_end..]
                .chars()
                .next()
                .is_none_or(|character| phrase_neighbor_is_valid(character, allow_cjk));
    }
    phrase_neighbor_is_valid(character, allow_cjk)
}

fn is_sentence_boundary_separator(character: char) -> bool {
    matches!(character, '.' | ':' | '．' | '：')
}

fn phrase_neighbor_is_valid(character: char, allow_cjk: bool) -> bool {
    if character == '_' || character.is_ascii_alphanumeric() || character.is_numeric() {
        return false;
    }
    allow_cjk || !character.is_alphanumeric()
}

fn is_structural_separator(character: char) -> bool {
    is_identifier_joiner(character) || matches!(character, '.' | ':' | '．' | '：')
}

pub(super) fn is_identifier_joiner(character: char) -> bool {
    matches!(
        character,
        '_' | '-' | '/' | '\\' | '＿' | '－' | '／' | '＼'
    )
}
