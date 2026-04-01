use crate::temporal::types::TemporalConstraint;

/// Try to extract a time range from query text. Returns None if no temporal expression found.
pub fn extract_temporal(query: &str) -> Option<TemporalConstraint> {
    let now = chrono::Utc::now().timestamp();
    let day = 86_400i64;
    let lower = query.to_lowercase();

    if lower.contains("yesterday") || lower.contains("昨天") {
        return Some(TemporalConstraint {
            start_epoch: now - day,
            end_epoch: now,
        });
    }
    if lower.contains("today") || lower.contains("今天") {
        let today_start = now - (now % day);
        return Some(TemporalConstraint {
            start_epoch: today_start,
            end_epoch: now,
        });
    }
    if lower.contains("last week") || lower.contains("上周") {
        return Some(TemporalConstraint {
            start_epoch: now - 7 * day,
            end_epoch: now,
        });
    }
    if lower.contains("last month") || lower.contains("上个月") || lower.contains("上月") {
        return Some(TemporalConstraint {
            start_epoch: now - 30 * day,
            end_epoch: now,
        });
    }
    if lower.contains("this week") || lower.contains("这周") || lower.contains("本周") {
        return Some(TemporalConstraint {
            start_epoch: now - 7 * day,
            end_epoch: now,
        });
    }
    if lower.contains("recently") || lower.contains("最近") {
        return Some(TemporalConstraint {
            start_epoch: now - 3 * day,
            end_epoch: now,
        });
    }
    if let Some(n) = parse_n_days_ago(&lower) {
        return Some(TemporalConstraint {
            start_epoch: now - n * day,
            end_epoch: now - (n - 1) * day,
        });
    }
    if let Some(n) = parse_last_n_days(&lower) {
        return Some(TemporalConstraint {
            start_epoch: now - n * day,
            end_epoch: now,
        });
    }

    None
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
