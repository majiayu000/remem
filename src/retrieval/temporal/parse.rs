use chrono::{Datelike, NaiveDate};

use crate::retrieval::temporal::types::{TemporalConstraint, TemporalField};

/// Try to extract a time range from query text. Returns None if no temporal expression found.
pub fn extract_temporal(query: &str) -> Option<TemporalConstraint> {
    let now = chrono::Utc::now().timestamp();
    let day = 86_400i64;
    let lower = query.to_lowercase();
    let field = temporal_field_for_query(&lower);

    if let Some((start_epoch, end_epoch)) = parse_exact_date_or_month(&lower) {
        return Some(TemporalConstraint {
            start_epoch,
            end_epoch,
            field,
        });
    }

    if lower.contains("yesterday") || lower.contains("昨天") {
        return Some(TemporalConstraint {
            start_epoch: now - day,
            end_epoch: now,
            field,
        });
    }
    if lower.contains("today") || lower.contains("今天") {
        let today_start = now - (now % day);
        return Some(TemporalConstraint {
            start_epoch: today_start,
            end_epoch: now,
            field,
        });
    }
    if lower.contains("last week") || lower.contains("上周") {
        return Some(TemporalConstraint {
            start_epoch: now - 7 * day,
            end_epoch: now,
            field,
        });
    }
    if lower.contains("last month") || lower.contains("上个月") || lower.contains("上月") {
        return Some(TemporalConstraint {
            start_epoch: now - 30 * day,
            end_epoch: now,
            field,
        });
    }
    if lower.contains("this week") || lower.contains("这周") || lower.contains("本周") {
        return Some(TemporalConstraint {
            start_epoch: now - 7 * day,
            end_epoch: now,
            field,
        });
    }
    if lower.contains("recently") || lower.contains("最近") {
        return Some(TemporalConstraint {
            start_epoch: now - 3 * day,
            end_epoch: now,
            field,
        });
    }
    if let Some(n) = parse_n_days_ago(&lower) {
        return Some(TemporalConstraint {
            start_epoch: now - n * day,
            end_epoch: now - (n - 1) * day,
            field,
        });
    }
    if let Some(n) = parse_last_n_days(&lower) {
        return Some(TemporalConstraint {
            start_epoch: now - n * day,
            end_epoch: now,
            field,
        });
    }

    None
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

fn parse_exact_date_or_month(lower: &str) -> Option<(i64, i64)> {
    parse_separated_ymd(lower)
        .or_else(|| parse_chinese_ymd(lower))
        .or_else(|| parse_month_name_date(lower))
}

fn parse_separated_ymd(lower: &str) -> Option<(i64, i64)> {
    for raw in lower.split_whitespace() {
        let token =
            raw.trim_matches(|c: char| !(c.is_ascii_digit() || c == '-' || c == '/' || c == '.'));
        for fmt in ["%Y-%m-%d", "%Y/%m/%d", "%Y.%m.%d"] {
            if let Ok(date) = NaiveDate::parse_from_str(token, fmt) {
                return day_range(date);
            }
        }
    }
    None
}

fn parse_chinese_ymd(lower: &str) -> Option<(i64, i64)> {
    let year_idx = lower.find('年')?;
    let year = parse_trailing_u32(&lower[..year_idx])? as i32;
    let after_year = &lower[year_idx + '年'.len_utf8()..];
    let month_idx = after_year.find('月')?;
    let month = parse_u32(after_year[..month_idx].trim())?;
    let after_month = &after_year[month_idx + '月'.len_utf8()..];
    let day_text = after_month.split(['日', '号']).next().unwrap_or("").trim();

    if let Some(day) = parse_leading_u32(day_text) {
        day_range(NaiveDate::from_ymd_opt(year, month, day)?)
    } else {
        month_range(year, month)
    }
}

fn parse_month_name_date(lower: &str) -> Option<(i64, i64)> {
    let parts: Vec<&str> = lower
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|part| !part.is_empty())
        .collect();
    let current_year = chrono::Utc::now().year();

    for (idx, part) in parts.iter().enumerate() {
        let month = match month_number(part) {
            Some(month) => month,
            None => continue,
        };
        let next = parts.get(idx + 1).copied();
        let next2 = parts.get(idx + 2).copied();
        let previous = idx.checked_sub(1).and_then(|prev| parts.get(prev).copied());

        if let (Some(day), Some(year)) = (previous.and_then(parse_day), next.and_then(parse_year)) {
            return day_range(NaiveDate::from_ymd_opt(year, month, day)?);
        }
        if let (Some(year), Some(day)) = (previous.and_then(parse_year), next.and_then(parse_day)) {
            return day_range(NaiveDate::from_ymd_opt(year, month, day)?);
        }
        if let Some(day) = next.and_then(parse_day) {
            let year = next2.and_then(parse_year).unwrap_or(current_year);
            return day_range(NaiveDate::from_ymd_opt(year, month, day)?);
        }
        if let Some(year) = next.and_then(parse_year) {
            return month_range(year, month);
        }
        if let Some(year) = previous.and_then(parse_year) {
            return month_range(year, month);
        }
        if let Some(day) = previous.and_then(parse_day) {
            let year = next.and_then(parse_year).unwrap_or(current_year);
            return day_range(NaiveDate::from_ymd_opt(year, month, day)?);
        }
    }

    None
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

fn parse_trailing_u32(input: &str) -> Option<u32> {
    let digits: String = input
        .chars()
        .rev()
        .take_while(|c| c.is_ascii_digit())
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    parse_u32(&digits)
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

fn parse_n_days_ago(lower: &str) -> Option<i64> {
    for word in lower.split_whitespace() {
        if let Ok(n) = word.parse::<i64>() {
            if lower.contains("days ago") || lower.contains("day ago") {
                return Some(n);
            }
        }
    }
    if lower.contains("天前") {
        let before_tian = lower.split("天前").next()?;
        let num_str: String = before_tian
            .chars()
            .rev()
            .take_while(|c| c.is_ascii_digit())
            .collect::<String>()
            .chars()
            .rev()
            .collect();
        if let Ok(n) = num_str.parse::<i64>() {
            return Some(n);
        }
        let last_char = before_tian.chars().last()?;
        return cn_digit(last_char);
    }
    None
}

fn parse_last_n_days(lower: &str) -> Option<i64> {
    if lower.contains("last") && lower.contains("days") {
        for word in lower.split_whitespace() {
            if let Ok(n) = word.parse::<i64>() {
                return Some(n);
            }
        }
    }
    if lower.contains("最近") && lower.contains('天') {
        let after = lower.split("最近").nth(1)?;
        let before_tian = after.split('天').next()?;
        if let Ok(n) = before_tian.trim().parse::<i64>() {
            return Some(n);
        }
        let c = before_tian.trim().chars().next()?;
        return cn_digit(c);
    }
    None
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
