use std::ops::Range;

use chrono::{Datelike, NaiveDate};

use crate::retrieval::temporal::types::{TemporalConstraint, TemporalField};

mod boundary;

use boundary::{
    cjk_day_phrase_span, date_span_with_context, has_cjk_phrase_context,
    has_cjk_temporal_introducer_before, has_phrase_context, has_recent_phrase_context,
    is_identifier_joiner, validated_temporal_span,
};

struct ParsedTemporal {
    constraint: TemporalConstraint,
    consumed_span: Range<usize>,
}

struct QueryWord<'a> {
    text: &'a str,
    span: Range<usize>,
}

/// Try to extract a time range from query text. Returns None if no temporal expression found.
pub fn extract_temporal(query: &str) -> Option<TemporalConstraint> {
    parse_temporal(query).map(|parsed| parsed.constraint)
}

impl TemporalConstraint {
    pub(crate) fn query_without_temporal_expression(query: &str) -> String {
        let Some(parsed) = parse_temporal(query) else {
            return query.to_string();
        };
        let mut semantic_query = query.to_string();
        semantic_query.replace_range(parsed.consumed_span, " ");
        semantic_query
    }
}

fn parse_temporal(query: &str) -> Option<ParsedTemporal> {
    let lower = query.to_ascii_lowercase();
    let mut candidate_query = lower.clone();
    for _ in 0..=lower.len() {
        let mut parsed = parse_temporal_lower(&candidate_query)?;
        if let Some(consumed_span) = validated_temporal_span(&lower, &parsed.consumed_span) {
            parsed.consumed_span = consumed_span;
            return Some(parsed);
        }

        let rejected_span = parsed.consumed_span;
        if rejected_span.is_empty() {
            return None;
        }
        let structural_mask = "_".repeat(rejected_span.len());
        candidate_query.replace_range(rejected_span, &structural_mask);
    }
    None
}

fn parse_temporal_lower(lower: &str) -> Option<ParsedTemporal> {
    let now = chrono::Utc::now().timestamp();
    let day = 86_400i64;
    let field = temporal_field_for_query(lower);

    if let Some((start_epoch, end_epoch, consumed_span)) = parse_exact_date_or_month(lower) {
        return Some(parsed_constraint(
            start_epoch,
            end_epoch,
            field,
            consumed_span,
        ));
    }

    if let Some(consumed_span) = first_phrase_span(lower, &["yesterday", "昨天"]) {
        return Some(parsed_constraint(now - day, now, field, consumed_span));
    }
    if let Some(consumed_span) = first_phrase_span(lower, &["today", "今天"]) {
        let today_start = now - (now % day);
        return Some(parsed_constraint(today_start, now, field, consumed_span));
    }
    if let Some(consumed_span) = first_phrase_span(lower, &["last week", "上周"]) {
        return Some(parsed_constraint(now - 7 * day, now, field, consumed_span));
    }
    if let Some(consumed_span) = first_phrase_span(lower, &["last month", "上个月", "上月"]) {
        return Some(parsed_constraint(now - 30 * day, now, field, consumed_span));
    }
    if let Some(consumed_span) = first_phrase_span(lower, &["this week", "这周", "本周"]) {
        return Some(parsed_constraint(now - 7 * day, now, field, consumed_span));
    }
    if let Some((n, consumed_span)) = parse_n_days_ago(lower) {
        let start_delta = n.checked_mul(day)?;
        let end_delta = n.checked_sub(1)?.checked_mul(day)?;
        return Some(parsed_constraint(
            now.checked_sub(start_delta)?,
            now.checked_sub(end_delta)?,
            field,
            consumed_span,
        ));
    }
    if let Some((n, consumed_span)) = parse_last_n_days(lower) {
        let delta = n.checked_mul(day)?;
        return Some(parsed_constraint(
            now.checked_sub(delta)?,
            now,
            field,
            consumed_span,
        ));
    }
    if has_quantified_recent_day_phrase(lower) {
        return None;
    }
    if let Some(consumed_span) = first_phrase_span(lower, &["recently", "最近"]) {
        return Some(parsed_constraint(now - 3 * day, now, field, consumed_span));
    }

    None
}

fn parsed_constraint(
    start_epoch: i64,
    end_epoch: i64,
    field: TemporalField,
    consumed_span: Range<usize>,
) -> ParsedTemporal {
    ParsedTemporal {
        constraint: TemporalConstraint {
            start_epoch,
            end_epoch,
            field,
        },
        consumed_span,
    }
}

fn first_phrase_span(query: &str, phrases: &[&str]) -> Option<Range<usize>> {
    phrases.iter().find_map(|phrase| {
        query.match_indices(phrase).find_map(|(start, _)| {
            let span = start..start + phrase.len();
            if matches!(*phrase, "昨天" | "今天") {
                return cjk_day_phrase_span(query, &span);
            }
            let is_ascii_phrase = phrase
                .chars()
                .any(|character| character.is_ascii_alphabetic());
            let valid_context = if is_ascii_phrase {
                has_phrase_context(query, &span)
            } else if *phrase == "最近" {
                has_recent_phrase_context(query, &span)
            } else {
                has_cjk_phrase_context(query, &span)
            };
            if !valid_context {
                return None;
            }
            Some(span)
        })
    })
}

fn temporal_field_for_query(lower: &str) -> TemporalField {
    if lower.contains("updated")
        || lower.contains("update")
        || lower.contains("changed")
        || lower.contains("modified")
        || lower.contains("mutation")
        || lower.contains("mutated")
        || lower.contains("更新")
        || lower.contains("修改")
        || lower.contains("变更")
    {
        TemporalField::UpdatedAt
    } else {
        TemporalField::EventTime
    }
}

fn parse_exact_date_or_month(lower: &str) -> Option<(i64, i64, Range<usize>)> {
    parse_separated_ymd(lower)
        .or_else(|| parse_chinese_ymd(lower))
        .or_else(|| parse_month_name_date(lower))
}

fn parse_separated_ymd(lower: &str) -> Option<(i64, i64, Range<usize>)> {
    let mut cursor = 0;
    'candidates: while cursor < lower.len() {
        let candidate_start = lower[cursor..]
            .char_indices()
            .find_map(|(offset, character)| {
                character.is_ascii_digit().then_some(cursor + offset)
            })?;
        let candidate_end = lower[candidate_start..]
            .char_indices()
            .find_map(|(offset, character)| {
                (!(character.is_ascii_digit()
                    || character == '-'
                    || character == '/'
                    || character == '.'))
                    .then_some(candidate_start + offset)
            })
            .unwrap_or(lower.len());
        cursor = candidate_end;
        let candidate = &lower[candidate_start..candidate_end];
        for token_end in candidate
            .char_indices()
            .filter_map(|(index, character)| {
                character
                    .is_ascii_digit()
                    .then_some(index + character.len_utf8())
            })
            .rev()
        {
            let token = &candidate[..token_end];
            for fmt in ["%Y-%m-%d", "%Y/%m/%d", "%Y.%m.%d"] {
                if let Ok(date) = NaiveDate::parse_from_str(token, fmt) {
                    let (start, end) = day_range(date)?;
                    let span = candidate_start..candidate_start + token_end;
                    let Some(consumed_span) = date_span_with_context(lower, &span) else {
                        continue 'candidates;
                    };
                    return Some((start, end, consumed_span));
                }
            }
        }
    }
    None
}

fn parse_chinese_ymd(lower: &str) -> Option<(i64, i64, Range<usize>)> {
    for (year_idx, _) in lower.match_indices('年') {
        let Some(year_text) = trailing_ascii_digits(&lower[..year_idx]) else {
            continue;
        };
        let year_start = year_idx - year_text.len();
        let Some(year) = parse_u32(year_text).map(|year| year as i32) else {
            continue;
        };
        let after_year = &lower[year_idx + '年'.len_utf8()..];
        let Some(month_idx) = after_year.find('月') else {
            continue;
        };
        let month_text = after_year[..month_idx].trim();
        let Some(month) = parse_u32(month_text) else {
            continue;
        };
        let month_end = year_idx + '年'.len_utf8() + month_idx + '月'.len_utf8();
        let after_month = &lower[month_end..];
        let day_text = after_month.split(['日', '号']).next().unwrap_or("").trim();

        let candidate = if let Some(day) = parse_leading_u32(day_text) {
            let Some(day_digits) = leading_ascii_digits(day_text) else {
                continue;
            };
            let Some(day_offset) = after_month.find(day_digits) else {
                continue;
            };
            let day_start = month_end + day_offset;
            let day_end = day_start + day_digits.len();
            let marker_len = lower[day_end..]
                .chars()
                .next()
                .filter(|marker| matches!(marker, '日' | '号'))
                .map_or(0, char::len_utf8);
            let Some(date) = NaiveDate::from_ymd_opt(year, month, day) else {
                continue;
            };
            let Some((start, end)) = day_range(date) else {
                continue;
            };
            (start, end, year_start..day_end + marker_len)
        } else {
            let Some((start, end)) = month_range(year, month) else {
                continue;
            };
            (start, end, year_start..month_end)
        };

        if let Some(consumed_span) = date_span_with_context(lower, &candidate.2) {
            return Some((candidate.0, candidate.1, consumed_span));
        }
    }

    None
}

fn parse_month_name_date(lower: &str) -> Option<(i64, i64, Range<usize>)> {
    let parts = query_words(lower);
    let current_year = chrono::Utc::now().year();

    for (idx, part) in parts.iter().enumerate() {
        let month = match month_number(part.text) {
            Some(month) => month,
            None => continue,
        };
        let next = parts.get(idx + 1);
        let next2 = parts.get(idx + 2);
        let previous = idx.checked_sub(1).and_then(|prev| parts.get(prev));

        let candidate = (|| {
            if let (Some(day), Some(year)) = (
                previous.and_then(|word| parse_day(word.text)),
                next.and_then(|word| parse_year(word.text)),
            ) {
                let (start, end) = day_range(NaiveDate::from_ymd_opt(year, month, day)?)?;
                return Some((start, end, previous?.span.start..next?.span.end));
            }
            if let (Some(year), Some(day)) = (
                previous.and_then(|word| parse_year(word.text)),
                next.and_then(|word| parse_day(word.text)),
            ) {
                let (start, end) = day_range(NaiveDate::from_ymd_opt(year, month, day)?)?;
                return Some((start, end, previous?.span.start..next?.span.end));
            }
            if let Some(day) = next.and_then(|word| parse_day(word.text)) {
                let explicit_year = next2.and_then(|word| parse_year(word.text));
                if next2.is_some_and(|word| {
                    word.text
                        .chars()
                        .next()
                        .is_some_and(|character| character.is_ascii_digit())
                        && explicit_year.is_none()
                }) {
                    return None;
                }
                let year = explicit_year.unwrap_or(current_year);
                let (start, end) = day_range(NaiveDate::from_ymd_opt(year, month, day)?)?;
                let last = if explicit_year.is_some() {
                    next2?
                } else {
                    next?
                };
                return Some((start, end, part.span.start..last.span.end));
            }
            if let Some(year) = next.and_then(|word| parse_year(word.text)) {
                let (start, end) = month_range(year, month)?;
                return Some((start, end, part.span.start..next?.span.end));
            }
            if let Some(year) = previous.and_then(|word| parse_year(word.text)) {
                let (start, end) = month_range(year, month)?;
                return Some((start, end, previous?.span.start..part.span.end));
            }
            if let Some(day) = previous.and_then(|word| parse_day(word.text)) {
                let year = next
                    .and_then(|word| parse_year(word.text))
                    .unwrap_or(current_year);
                let (start, end) = day_range(NaiveDate::from_ymd_opt(year, month, day)?)?;
                let last = next
                    .filter(|word| parse_year(word.text).is_some())
                    .unwrap_or(part);
                return Some((start, end, previous?.span.start..last.span.end));
            }
            None
        })();

        if candidate.as_ref().is_some_and(|(_, _, span)| {
            has_phrase_context(lower, span) && has_natural_date_word_gaps(lower, &parts, span)
        }) {
            return candidate;
        }
    }

    None
}

fn has_natural_date_word_gaps(query: &str, words: &[QueryWord<'_>], span: &Range<usize>) -> bool {
    let candidate_words: Vec<_> = words
        .iter()
        .filter(|word| word.span.start >= span.start && word.span.end <= span.end)
        .collect();
    candidate_words.windows(2).all(|pair| {
        let gap = &query[pair[0].span.end..pair[1].span.start];
        if !gap.is_empty() && gap.chars().all(char::is_whitespace) {
            return true;
        }
        if let Some(rest) = gap.strip_prefix(['.', '．']) {
            return is_abbreviated_month_name(pair[0].text)
                && !rest.is_empty()
                && rest.chars().all(char::is_whitespace);
        }
        let Some(rest) = gap.strip_prefix([',', '，']) else {
            return false;
        };
        rest.chars().all(char::is_whitespace)
            && (!rest.is_empty() || parse_year(pair[1].text).is_some())
    })
}

fn is_abbreviated_month_name(input: &str) -> bool {
    matches!(
        input,
        "jan"
            | "feb"
            | "mar"
            | "apr"
            | "jun"
            | "jul"
            | "aug"
            | "sep"
            | "sept"
            | "oct"
            | "nov"
            | "dec"
    )
}

fn day_range(date: NaiveDate) -> Option<(i64, i64)> {
    let start = date.and_hms_opt(0, 0, 0)?.and_utc().timestamp();
    Some((start, start + 86_400 - 1))
}

fn month_range(year: i32, month: u32) -> Option<(i64, i64)> {
    let start_date = NaiveDate::from_ymd_opt(year, month, 1)?;
    let (next_year, next_month) = if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    };
    let next_date = NaiveDate::from_ymd_opt(next_year, next_month, 1)?;
    let start = start_date.and_hms_opt(0, 0, 0)?.and_utc().timestamp();
    let next = next_date.and_hms_opt(0, 0, 0)?.and_utc().timestamp();
    Some((start, next - 1))
}

fn trailing_ascii_digits(input: &str) -> Option<&str> {
    let start = input
        .char_indices()
        .rev()
        .take_while(|(_, c)| c.is_ascii_digit())
        .last()
        .map(|(idx, _)| idx)?;
    Some(&input[start..])
}

fn leading_ascii_digits(input: &str) -> Option<&str> {
    let end = input
        .char_indices()
        .take_while(|(_, c)| c.is_ascii_digit())
        .last()
        .map(|(idx, c)| idx + c.len_utf8())?;
    Some(&input[..end])
}

fn parse_year(input: &str) -> Option<i32> {
    let year = parse_u32(input)?;
    if (1000..=9999).contains(&year) {
        Some(year as i32)
    } else {
        None
    }
}

fn parse_day(input: &str) -> Option<u32> {
    let day = parse_u32(input)?;
    if (1..=31).contains(&day) {
        Some(day)
    } else {
        None
    }
}

fn parse_u32(input: &str) -> Option<u32> {
    input.trim().parse::<u32>().ok()
}

fn parse_leading_u32(input: &str) -> Option<u32> {
    let digits: String = input
        .trim_start()
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    parse_u32(&digits)
}

fn month_number(input: &str) -> Option<u32> {
    match input {
        "jan" | "january" => Some(1),
        "feb" | "february" => Some(2),
        "mar" | "march" => Some(3),
        "apr" | "april" => Some(4),
        "may" => Some(5),
        "jun" | "june" => Some(6),
        "jul" | "july" => Some(7),
        "aug" | "august" => Some(8),
        "sep" | "sept" | "september" => Some(9),
        "oct" | "october" => Some(10),
        "nov" | "november" => Some(11),
        "dec" | "december" => Some(12),
        _ => None,
    }
}

fn parse_n_days_ago(lower: &str) -> Option<(i64, Range<usize>)> {
    let words = query_words(lower);
    for window in words.windows(3) {
        let unit = window[1].text;
        let ago = window[2].text;
        let span = window[0].span.start..window[2].span.end;
        if matches!(unit, "day" | "days")
            && ago == "ago"
            && has_whitespace_word_gaps(lower, window)
            && has_phrase_context(lower, &span)
        {
            if let Some(n) = positive_query_day_count(lower, &window[0]) {
                return Some((n, span));
            }
        }
    }
    for (suffix_start, _) in lower.match_indices("天前") {
        let suffix_end = suffix_start + "天前".len();
        let before_tian = &lower[..suffix_start];
        if let Some(num_str) = trailing_ascii_digits(before_tian) {
            let number_start = suffix_start - num_str.len();
            let span = number_start..suffix_end;
            if has_invalid_numeric_left_boundary(lower, number_start)
                || has_cn_number_component_prefix(lower, number_start)
                || !has_cjk_phrase_context(lower, &span)
            {
                continue;
            }
            if let Some(n) = positive_day_count(num_str) {
                return Some((n, span));
            }
        }
        let Some((number_start, last_char)) = before_tian.char_indices().last() else {
            continue;
        };
        let span = number_start..suffix_end;
        if has_numeric_sign_prefix(lower, number_start)
            || has_cn_number_component_prefix(lower, number_start)
            || has_invalid_numeric_left_boundary(lower, number_start)
            || !has_cjk_phrase_context(lower, &span)
        {
            continue;
        }
        if let Some(n) = cn_digit(last_char) {
            return Some((n, span));
        }
    }
    None
}

fn parse_last_n_days(lower: &str) -> Option<(i64, Range<usize>)> {
    let words = query_words(lower);
    for window in words.windows(3) {
        let last = window[0].text;
        let unit = window[2].text;
        let span = window[0].span.start..window[2].span.end;
        if last == "last"
            && matches!(unit, "day" | "days")
            && has_whitespace_word_gaps(lower, window)
            && has_phrase_context(lower, &span)
        {
            if let Some(n) = positive_query_day_count(lower, &window[1]) {
                return Some((n, span));
            }
        }
    }
    for (start, _) in lower.match_indices("最近") {
        let after_start = start + "最近".len();
        let after = &lower[after_start..];
        let Some(tian_start) = after.find('天') else {
            continue;
        };
        let before_tian = &after[..tian_start];
        let end = after_start + tian_start + '天'.len_utf8();
        let span = start..end;
        if !has_cjk_phrase_context(lower, &span) {
            continue;
        }
        if let Some(n) = positive_day_count(before_tian.trim()) {
            return Some((n, span));
        }
        let Ok(digit) = before_tian.trim().parse::<char>() else {
            continue;
        };
        if let Some(n) = cn_digit(digit) {
            return Some((n, span));
        }
    }
    None
}

fn has_whitespace_word_gaps(query: &str, words: &[QueryWord<'_>]) -> bool {
    words.windows(2).all(|pair| {
        let gap = &query[pair[0].span.end..pair[1].span.start];
        !gap.is_empty() && gap.chars().all(char::is_whitespace)
    })
}

fn query_words(query: &str) -> Vec<QueryWord<'_>> {
    let mut words = Vec::new();
    let mut start = None;
    for (index, character) in query.char_indices() {
        if character.is_alphanumeric() || character == '_' {
            let ascii_to_cjk_boundary = !character.is_ascii()
                && start.is_some_and(|word_start| {
                    query[word_start..index]
                        .chars()
                        .all(|word_character| word_character.is_ascii_alphanumeric())
                });
            let cjk_introducer_boundary = character.is_ascii_alphanumeric()
                && start.is_some()
                && has_cjk_temporal_introducer_before(query, index);
            if ascii_to_cjk_boundary || cjk_introducer_boundary {
                if let Some(word_start) = start.replace(index) {
                    words.push(QueryWord {
                        text: &query[word_start..index],
                        span: word_start..index,
                    });
                    continue;
                }
            }
            start.get_or_insert(index);
        } else if let Some(start) = start.take() {
            words.push(QueryWord {
                text: &query[start..index],
                span: start..index,
            });
        }
    }
    if let Some(start) = start {
        words.push(QueryWord {
            text: &query[start..],
            span: start..query.len(),
        });
    }
    words
}

fn positive_day_count(text: &str) -> Option<i64> {
    text.parse::<i64>().ok().filter(|count| *count > 0)
}

fn positive_query_day_count(query: &str, word: &QueryWord<'_>) -> Option<i64> {
    (!has_invalid_numeric_left_boundary(query, word.span.start)
        && !has_invalid_numeric_right_boundary(query, word.span.end))
    .then(|| positive_day_count(word.text))
    .flatten()
}

fn has_numeric_sign_prefix(query: &str, start: usize) -> bool {
    preceding_separator(query, start)
        .0
        .chars()
        .any(is_numeric_sign)
}

fn has_invalid_numeric_left_boundary(query: &str, start: usize) -> bool {
    let (separator, preceding) = preceding_separator(query, start);
    if separator
        .chars()
        .any(|character| is_numeric_sign(character) || is_identifier_joiner(character))
    {
        return true;
    }
    let has_whitespace_boundary = separator
        .chars()
        .any(|character| character.is_whitespace() && !is_numeric_grouping_space(character));
    let has_dash_boundary = separator
        .chars()
        .any(|character| matches!(character, '—' | '–'));
    let has_clause_boundary = separator.chars().any(|character| {
        matches!(
            character,
            ',' | ';' | '!' | '?' | '(' | '，' | '。' | '；' | '！' | '？' | '、' | '（'
        )
    });
    if separator
        .chars()
        .any(|character| matches!(character, ',' | '，'))
        && !has_whitespace_boundary
        && preceding.is_some_and(char::is_numeric)
    {
        let separator_start = start - separator.len();
        return preceding_identifier_run(query, separator_start)
            .is_none_or(|run| !run.chars().any(char::is_alphabetic));
    }
    !has_whitespace_boundary
        && !has_dash_boundary
        && !has_clause_boundary
        && preceding.is_some_and(char::is_numeric)
}

fn preceding_identifier_run(query: &str, end: usize) -> Option<&str> {
    let before = &query[..end];
    let start = before
        .char_indices()
        .rev()
        .take_while(|(_, character)| {
            character.is_alphanumeric() || is_identifier_joiner(*character)
        })
        .last()
        .map(|(index, _)| index)?;
    Some(&before[start..])
}

fn has_invalid_numeric_right_boundary(query: &str, end: usize) -> bool {
    query[end..]
        .chars()
        .take_while(|character| !character.is_alphanumeric())
        .any(is_numeric_sign)
}

fn preceding_separator(query: &str, start: usize) -> (&str, Option<char>) {
    let before = &query[..start];
    let separator_start = before
        .char_indices()
        .rev()
        .find(|(_, character)| character.is_alphanumeric())
        .map_or(0, |(index, character)| index + character.len_utf8());
    (
        &query[separator_start..start],
        query[..separator_start].chars().next_back(),
    )
}

fn is_numeric_sign(character: char) -> bool {
    matches!(character, '+' | '＋' | '﹢' | '-' | '−' | '－' | '﹣')
}

fn is_numeric_grouping_space(character: char) -> bool {
    matches!(character, '\u{a0}' | '\u{202f}' | '\u{2007}')
}

fn has_cn_number_component_prefix(query: &str, start: usize) -> bool {
    query[..start]
        .chars()
        .next_back()
        .is_some_and(is_cn_number_component)
}

pub(super) fn is_cn_number_component(character: char) -> bool {
    "零〇一二两兩三四五六七八九十拾百佰千仟万萬亿億壹贰貳叁參肆伍陆陸柒捌玖".contains(character)
}

fn has_quantified_recent_day_phrase(query: &str) -> bool {
    query.match_indices("最近").any(|(start, _)| {
        let recent_span = start..start + "最近".len();
        if !has_recent_phrase_context(query, &recent_span) {
            return false;
        }
        let Some((count, _)) = query[recent_span.end..].split_once('天') else {
            return false;
        };
        let count = count.trim();
        let count = count
            .strip_prefix('v')
            .or_else(|| count.strip_prefix('V'))
            .unwrap_or(count);
        is_numeric_expression(count)
            || ['到', '至'].into_iter().any(|separator| {
                count.split_once(separator).is_some_and(|(start, end)| {
                    is_numeric_expression(start) && is_numeric_expression(end)
                })
            })
    })
}

fn is_numeric_expression(count: &str) -> bool {
    let mut has_number = false;
    let valid = count.chars().all(|character| {
        if character.is_numeric() || is_cn_number_component(character) {
            has_number = true;
            true
        } else {
            character.is_whitespace() || !character.is_alphanumeric()
        }
    });
    has_number && valid
}

fn cn_digit(c: char) -> Option<i64> {
    match c {
        '一' | '壹' => Some(1),
        '二' | '两' | '贰' => Some(2),
        '三' | '叁' => Some(3),
        '四' | '肆' => Some(4),
        '五' | '伍' => Some(5),
        '六' | '陆' => Some(6),
        '七' | '柒' => Some(7),
        '八' | '捌' => Some(8),
        '九' | '玖' => Some(9),
        '十' | '拾' => Some(10),
        _ => None,
    }
}
