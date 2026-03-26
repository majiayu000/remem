/// Time expression parser for temporal-aware retrieval.
/// Extracts time constraints from queries like "yesterday", "上周", "3 days ago".

pub struct TemporalConstraint {
    pub start_epoch: i64,
    pub end_epoch: i64,
}

/// Try to extract a time range from query text. Returns None if no temporal expression found.
pub fn extract_temporal(query: &str) -> Option<TemporalConstraint> {
    let now = chrono::Utc::now().timestamp();
    let day = 86400i64;
    let lower = query.to_lowercase();

    // English patterns
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

    // "N days ago" / "N天前"
    if let Some(n) = parse_n_days_ago(&lower) {
        return Some(TemporalConstraint {
            start_epoch: now - n * day,
            end_epoch: now - (n - 1) * day,
        });
    }

    // "last N days" / "最近N天"
    if let Some(n) = parse_last_n_days(&lower) {
        return Some(TemporalConstraint {
            start_epoch: now - n * day,
            end_epoch: now,
        });
    }

    None
}

fn parse_n_days_ago(lower: &str) -> Option<i64> {
    // "3 days ago", "7 days ago"
    for word in lower.split_whitespace() {
        if let Ok(n) = word.parse::<i64>() {
            if lower.contains("days ago") || lower.contains("day ago") {
                return Some(n);
            }
        }
    }
    // "三天前", "7天前"
    if lower.contains("天前") {
        let before_tian = lower.split("天前").next()?;
        // Try parsing the last number/character
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
        // Chinese numerals
        let last_char = before_tian.chars().last()?;
        return cn_digit(last_char);
    }
    None
}

fn parse_last_n_days(lower: &str) -> Option<i64> {
    // "last 7 days"
    if lower.contains("last") && lower.contains("days") {
        for word in lower.split_whitespace() {
            if let Ok(n) = word.parse::<i64>() {
                return Some(n);
            }
        }
    }
    // "最近7天" / "最近三天"
    if lower.contains("最近") && lower.contains("天") {
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

/// Search memories within a time range, sorted by recency.
pub fn search_by_time(
    conn: &rusqlite::Connection,
    constraint: &TemporalConstraint,
    project: Option<&str>,
    limit: i64,
) -> anyhow::Result<Vec<i64>> {
    let mut ids = Vec::new();

    if let Some(proj) = project {
        let mut stmt = conn.prepare(
            "SELECT id FROM memories
             WHERE status = 'active'
             AND project = ?3
             AND updated_at_epoch BETWEEN ?1 AND ?2
             ORDER BY updated_at_epoch DESC LIMIT ?4",
        )?;
        let rows = stmt.query_map(
            rusqlite::params![constraint.start_epoch, constraint.end_epoch, proj, limit],
            |r| r.get::<_, i64>(0),
        )?;
        for row in rows {
            ids.push(row?);
        }
    } else {
        let mut stmt = conn.prepare(
            "SELECT id FROM memories
             WHERE status = 'active'
             AND updated_at_epoch BETWEEN ?1 AND ?2
             ORDER BY updated_at_epoch DESC LIMIT ?3",
        )?;
        let rows = stmt.query_map(
            rusqlite::params![constraint.start_epoch, constraint.end_epoch, limit],
            |r| r.get::<_, i64>(0),
        )?;
        for row in rows {
            ids.push(row?);
        }
    }

    Ok(ids)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_yesterday() {
        assert!(extract_temporal("yesterday's decisions").is_some());
        assert!(extract_temporal("昨天的决策").is_some());
    }

    #[test]
    fn parse_last_week() {
        assert!(extract_temporal("last week we discussed").is_some());
        assert!(extract_temporal("上周讨论的").is_some());
    }

    #[test]
    fn parse_n_days_ago_en() {
        let c = extract_temporal("3 days ago").unwrap();
        let now = chrono::Utc::now().timestamp();
        assert!((now - c.start_epoch - 3 * 86400).abs() < 2);
    }

    #[test]
    fn parse_n_days_ago_cn() {
        assert!(extract_temporal("三天前").is_some());
        assert!(extract_temporal("7天前").is_some());
    }

    #[test]
    fn parse_recently() {
        assert!(extract_temporal("最近的修改").is_some());
        assert!(extract_temporal("recently changed").is_some());
    }

    #[test]
    fn no_temporal_in_normal_query() {
        assert!(extract_temporal("FTS5 search optimization").is_none());
        assert!(extract_temporal("数据库加密").is_none());
    }
}
