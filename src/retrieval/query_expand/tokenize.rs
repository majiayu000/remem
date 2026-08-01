use super::translations::CJK_EN_TRANSLATIONS;

const CJK_QUERY_SEGMENTS: &[&str] = &[
    "什么时候",
    "为什么",
    "当前",
    "负责",
    "何时",
    "删除",
    "哪里",
    "哪些",
    "哪个",
    "如何",
    "怎么",
    "拥有",
    "替代",
    "使用",
    "维护",
    "为何",
    "什么",
    "影响",
    "验证",
    "修复",
    "最近",
    "目前",
    "阻塞",
    "与",
    "了",
    "吗",
    "和",
    "呢",
    "在",
    "是",
    "由",
    "的",
    "谁",
];

const CJK_COMPACT_IDENTIFIER_STOP_SEGMENTS: &[&str] = &[
    "与",
    "了",
    "吗",
    "和",
    "呢",
    "在",
    "是",
    "由",
    "的",
    "谁",
    "于",
    "自",
    "从",
    "至",
    "到",
    "截至",
    "截止",
    "截至到",
    "截止到",
    "自从",
    "早在",
    "直到",
];

const TERMINAL_CJK_QUERY_SEGMENTS: &[&str] = &["了", "吗", "呢"];
const INFIX_CJK_QUERY_SEGMENTS: &[&str] = &["由"];

pub(super) fn is_cjk(c: char) -> bool {
    matches!(
        c,
        '\u{4E00}'..='\u{9FFF}' |
        '\u{3400}'..='\u{4DBF}' |
        '\u{F900}'..='\u{FAFF}'
    )
}

pub(super) fn tokenize_mixed(raw: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    for part in raw.split_whitespace() {
        let chars: Vec<char> = part.chars().collect();
        if chars.is_empty() {
            continue;
        }
        let mut part_tokens = Vec::new();
        let mut i = 0;
        while i < chars.len() {
            if is_cjk(chars[i]) {
                let start = i;
                while i < chars.len() && is_cjk(chars[i]) {
                    i += 1;
                }
                part_tokens.push(chars[start..i].iter().collect::<String>());
            } else {
                let start = i;
                while i < chars.len() && !is_cjk(chars[i]) {
                    i += 1;
                }
                let segment: String = chars[start..i].iter().collect();
                let trimmed = segment.trim();
                if !trimmed.is_empty() {
                    part_tokens.push(trimmed.to_string());
                }
            }
        }
        tokens.extend(part_tokens.iter().cloned());
        tokens.extend(
            part_tokens
                .windows(2)
                .filter_map(|pair| compact_mixed_identifier(&pair[0], &pair[1])),
        );
    }
    tokens
}

fn compact_mixed_identifier(ascii: &str, cjk: &str) -> Option<String> {
    (is_short_ascii_identifier(ascii) && is_short_cjk_identifier_suffix(cjk))
        .then(|| format!("{ascii}{cjk}"))
}

fn is_short_ascii_identifier(segment: &str) -> bool {
    let length = segment.chars().count();
    (1..=2).contains(&length)
        && segment
            .chars()
            .all(|character| character.is_ascii_alphanumeric())
        && segment
            .chars()
            .any(|character| character.is_ascii_uppercase() || character.is_ascii_digit())
}

fn is_short_cjk_identifier_suffix(segment: &str) -> bool {
    let length = segment.chars().count();
    (1..=2).contains(&length)
        && segment.chars().all(is_cjk)
        && !CJK_COMPACT_IDENTIFIER_STOP_SEGMENTS.contains(&segment)
}

pub(super) fn segment_cjk(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut segments = Vec::new();
    let mut i = 0;

    while i < chars.len() {
        if let Some(best_len) = known_segment_len(&chars, i) {
            segments.push(chars[i..i + best_len].iter().collect());
            i += best_len;
        } else {
            let start = i;
            i += 1;
            while i < chars.len() && known_segment_len_after_unknown(&chars, start, i).is_none() {
                i += 1;
            }
            segments.push(chars[start..i].iter().collect());
        }
    }

    segments
}

fn known_segment_len_after_unknown(
    chars: &[char],
    unknown_start: usize,
    start: usize,
) -> Option<usize> {
    let length = known_segment_len(chars, start)?;
    let candidate: String = chars[start..start + length].iter().collect();
    (length > 1
        || known_segment_len(chars, start + length).is_some()
        || (start.saturating_sub(unknown_start) >= 2
            && INFIX_CJK_QUERY_SEGMENTS.contains(&candidate.as_str())
            && has_later_known_multi_segment(chars, start + length))
        || (start + length == chars.len()
            && TERMINAL_CJK_QUERY_SEGMENTS.contains(&candidate.as_str())))
    .then_some(length)
}

fn has_later_known_multi_segment(chars: &[char], start: usize) -> bool {
    (start..chars.len()).any(|index| {
        known_segment_len(chars, index).is_some_and(|segment_length| segment_length > 1)
    })
}

fn known_segment_len(chars: &[char], start: usize) -> Option<usize> {
    (1..=4).rev().find(|len| {
        if start + len > chars.len() {
            return false;
        }
        let candidate: String = chars[start..start + len].iter().collect();
        CJK_EN_TRANSLATIONS.contains_key(candidate.as_str())
            || CJK_QUERY_SEGMENTS.contains(&candidate.as_str())
    })
}
