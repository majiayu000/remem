use std::ops::Range;

use super::{is_cn_number_component, leading_ascii_digits};

const CJK_TEMPORAL_INTRODUCERS: &[&str] = &[
    "截至到",
    "截止到",
    "截至",
    "截止",
    "自从",
    "早在",
    "直到",
    "在",
    "于",
    "自",
    "从",
    "至",
    "到",
];

#[derive(Clone, Copy, Eq, PartialEq)]
enum CjkIntroducerBoundary {
    Valid,
    AmbiguousCjk,
    Structural,
}

pub(super) fn has_phrase_context(query: &str, span: &Range<usize>) -> bool {
    phrase_left_boundary_is_valid(query, span.start, false)
        && phrase_right_boundary_is_valid(query, span.end, true)
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
        Some(character) if character.is_alphanumeric() => {
            has_cjk_temporal_introducer_before(query, span.start)
        }
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

pub(super) fn validated_temporal_span(query: &str, span: &Range<usize>) -> Option<Range<usize>> {
    let left = query[..span.start].trim_end_matches(char::is_whitespace);
    let has_introducer_gap = left.len() < span.start;
    let mut consumed_span = span.clone();
    if let Some((introducer_start, boundary)) =
        cjk_temporal_introducer_candidate_before(query, left.len())
    {
        match boundary {
            CjkIntroducerBoundary::Valid => consumed_span.start = introducer_start,
            CjkIntroducerBoundary::Structural if !has_introducer_gap => return None,
            CjkIntroducerBoundary::AmbiguousCjk | CjkIntroducerBoundary::Structural => {}
        }
    }
    if left_separator_contains_identifier_joiner(query, consumed_span.start) {
        return None;
    }
    Some(consumed_span)
}

fn cjk_clock_time_suffix_end(query: &str, start: usize) -> Option<usize> {
    let start = cjk_clock_time_suffix_start(query, start)?;
    let (hour, hour_len) = leading_clock_hour(&query[start..])?;
    if hour > 23 {
        return None;
    }

    let hour_end = start + hour_len;
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

fn leading_clock_hour(input: &str) -> Option<(u32, usize)> {
    if let Some(hour_text) = leading_ascii_digits(input) {
        return Some((hour_text.parse::<u32>().ok()?, hour_text.len()));
    }
    let end = input
        .char_indices()
        .take_while(|(_, character)| is_cn_number_component(*character))
        .last()
        .map(|(index, character)| index + character.len_utf8())?;
    Some((parse_cjk_hour(&input[..end])?, end))
}

fn parse_cjk_hour(input: &str) -> Option<u32> {
    let chars: Vec<_> = input.chars().collect();
    match chars.as_slice() {
        [digit] => cjk_clock_digit(*digit),
        [ten, digit] if is_cjk_ten(*ten) => Some(10 + cjk_clock_digit(*digit)?),
        [digit, ten] if is_cjk_ten(*ten) && cjk_clock_digit(*digit)? == 2 => Some(20),
        [digit, ten, unit]
            if is_cjk_ten(*ten)
                && cjk_clock_digit(*digit)? == 2
                && cjk_clock_digit(*unit)? <= 3 =>
        {
            Some(20 + cjk_clock_digit(*unit)?)
        }
        _ => None,
    }
}

fn cjk_clock_digit(character: char) -> Option<u32> {
    match character {
        '零' | '〇' => Some(0),
        '一' | '壹' => Some(1),
        '二' | '两' | '兩' | '贰' | '貳' => Some(2),
        '三' | '叁' | '參' => Some(3),
        '四' | '肆' => Some(4),
        '五' | '伍' => Some(5),
        '六' | '陆' | '陸' => Some(6),
        '七' | '柒' => Some(7),
        '八' | '捌' => Some(8),
        '九' | '玖' => Some(9),
        '十' | '拾' => Some(10),
        _ => None,
    }
}

fn is_cjk_ten(character: char) -> bool {
    matches!(character, '十' | '拾')
}

fn phrase_left_boundary_is_valid(query: &str, start: usize, allow_cjk: bool) -> bool {
    let Some((_, character)) = query[..start].char_indices().next_back() else {
        return true;
    };
    if is_structural_separator(character) {
        return false;
    }
    if !allow_cjk && has_cjk_temporal_introducer_before(query, start) {
        return true;
    }
    phrase_neighbor_is_valid(character, allow_cjk)
}

pub(super) fn has_cjk_temporal_introducer_before(query: &str, start: usize) -> bool {
    cjk_temporal_introducer_start_before(query, start).is_some()
}

fn cjk_temporal_introducer_start_before(query: &str, end: usize) -> Option<usize> {
    cjk_temporal_introducer_candidate_before(query, end)
        .and_then(|(start, boundary)| (boundary == CjkIntroducerBoundary::Valid).then_some(start))
}

fn cjk_temporal_introducer_candidate_before(
    query: &str,
    end: usize,
) -> Option<(usize, CjkIntroducerBoundary)> {
    let left = &query[..end];
    CJK_TEMPORAL_INTRODUCERS.iter().find_map(|introducer| {
        let start = left.strip_suffix(introducer).map(|prefix| prefix.len())?;
        Some((
            start,
            cjk_temporal_introducer_left_boundary(query, start, introducer),
        ))
    })
}

fn left_separator_contains_identifier_joiner(query: &str, start: usize) -> bool {
    let before = &query[..start];
    let separator_start = before
        .char_indices()
        .rev()
        .find(|(_, character)| character.is_alphanumeric())
        .map_or(0, |(index, character)| index + character.len_utf8());
    before[separator_start..].chars().any(is_identifier_joiner)
}

fn cjk_temporal_introducer_left_boundary(
    query: &str,
    start: usize,
    introducer: &str,
) -> CjkIntroducerBoundary {
    let Some(character) = query[..start].chars().next_back() else {
        return CjkIntroducerBoundary::Valid;
    };
    if is_structural_separator(character) {
        return CjkIntroducerBoundary::Structural;
    }
    if character.is_whitespace() || !character.is_alphanumeric() {
        return CjkIntroducerBoundary::Valid;
    }
    if character.is_ascii_alphanumeric() || character.is_numeric() || introducer.chars().count() > 1
    {
        CjkIntroducerBoundary::Valid
    } else {
        CjkIntroducerBoundary::AmbiguousCjk
    }
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
