use super::synonyms::SYNONYMS;

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
        let mut i = 0;
        while i < chars.len() {
            if is_cjk(chars[i]) {
                let start = i;
                while i < chars.len() && is_cjk(chars[i]) {
                    i += 1;
                }
                tokens.push(chars[start..i].iter().collect());
            } else {
                let start = i;
                while i < chars.len() && !is_cjk(chars[i]) {
                    i += 1;
                }
                let segment: String = chars[start..i].iter().collect();
                let trimmed = segment.trim();
                if !trimmed.is_empty() {
                    tokens.push(trimmed.to_string());
                }
            }
        }
    }
    tokens
}

pub(super) fn segment_cjk(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut segments = Vec::new();
    let mut i = 0;

    while i < chars.len() {
        let mut best_len = 0;
        for len in (2..=4).rev() {
            if i + len <= chars.len() {
                let candidate: String = chars[i..i + len].iter().collect();
                if SYNONYMS.contains_key(candidate.as_str()) {
                    best_len = len;
                    break;
                }
            }
        }
        if best_len > 0 {
            segments.push(chars[i..i + best_len].iter().collect());
            i += best_len;
        } else {
            segments.push(chars[i..i + 1].iter().collect());
            i += 1;
        }
    }

    segments
}
