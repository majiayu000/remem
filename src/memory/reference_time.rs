pub(crate) fn contains_relative_time_reference(text: &str) -> bool {
    let lower = text.to_lowercase();
    let phrases = [
        "last week",
        "next week",
        "last month",
        "next month",
        "last year",
        "next year",
        "days ago",
        "day ago",
        "hours ago",
        "hour ago",
        "minutes ago",
        "minute ago",
    ];
    if phrases.iter().any(|phrase| lower.contains(phrase)) {
        return true;
    }
    ["today", "tomorrow", "yesterday"]
        .iter()
        .any(|word| contains_ascii_word(&lower, word))
        || [
            "今天", "明天", "昨天", "前天", "后天", "上周", "下周", "刚才", "刚刚",
        ]
        .iter()
        .any(|word| text.contains(word))
}

fn contains_ascii_word(haystack: &str, needle: &str) -> bool {
    haystack
        .match_indices(needle)
        .any(|(start, _)| is_ascii_word_boundary(haystack, start, needle.len()))
}

fn is_ascii_word_boundary(text: &str, start: usize, len: usize) -> bool {
    let before = start
        .checked_sub(1)
        .and_then(|idx| text.as_bytes().get(idx))
        .copied();
    let after = text.as_bytes().get(start + len).copied();
    !before.is_some_and(is_ascii_word_byte) && !after.is_some_and(is_ascii_word_byte)
}

fn is_ascii_word_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

pub(crate) fn parse_json_epoch_value(value: Option<&serde_json::Value>) -> Option<i64> {
    let value = value?;
    if let Some(epoch) = value.as_i64() {
        return Some(normalize_epoch_seconds(epoch));
    }
    let text = value.as_str()?.trim();
    if text.is_empty() {
        return None;
    }
    if let Ok(epoch) = text.parse::<i64>() {
        return Some(normalize_epoch_seconds(epoch));
    }
    chrono::DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|datetime| datetime.timestamp())
}

fn normalize_epoch_seconds(epoch: i64) -> i64 {
    if epoch.abs() >= 10_000_000_000 {
        epoch / 1000
    } else {
        epoch
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_relative_dates_narrowly() {
        assert!(contains_relative_time_reference("yesterday we changed it"));
        assert!(contains_relative_time_reference("昨天修好了"));
        assert!(contains_relative_time_reference("3 days ago"));
        assert!(!contains_relative_time_reference(
            "absolute date 2026-06-12"
        ));
        assert!(!contains_relative_time_reference("notoday is one token"));
    }

    #[test]
    fn parses_iso_and_epoch_reference_times() {
        assert_eq!(
            parse_json_epoch_value(Some(&serde_json::json!("2026-06-12T00:00:01.000Z"))),
            Some(1_781_222_401)
        );
        assert_eq!(
            parse_json_epoch_value(Some(&serde_json::json!(1_749_686_401_000_i64))),
            Some(1_749_686_401)
        );
    }
}
