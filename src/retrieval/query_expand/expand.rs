use std::collections::HashSet;

use super::tokenize::{is_cjk, segment_cjk, tokenize_mixed};
use super::translations::CJK_EN_TRANSLATIONS;

pub fn core_tokens(raw: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut seen = HashSet::new();

    let mixed_tokens = tokenize_mixed(raw);
    for token in &mixed_tokens {
        push_core_token(token, &mut tokens, &mut seen);
    }

    tokens
}

pub fn expand_query(raw: &str) -> Vec<String> {
    let mut expanded = Vec::new();
    let mut seen = HashSet::new();

    let mixed_tokens = tokenize_mixed(raw);
    for token in &mixed_tokens {
        let chars: Vec<char> = token.chars().collect();
        let all_cjk = !chars.is_empty() && chars.iter().all(|c| is_cjk(*c));

        if all_cjk && chars.len() > 1 {
            let segments = segment_cjk(token);
            let any_multi = segments.iter().any(|segment| segment.chars().count() > 1);

            if any_multi {
                for segment in &segments {
                    add_with_translations(segment, &mut expanded, &mut seen);
                }
                if seen.insert(token.to_lowercase()) {
                    expanded.push(token.to_string());
                }
            } else {
                add_with_translations(token, &mut expanded, &mut seen);
            }
        } else {
            add_with_translations(token, &mut expanded, &mut seen);
        }
    }

    expanded
}

fn push_core_token(token: &str, tokens: &mut Vec<String>, seen: &mut HashSet<String>) {
    let chars: Vec<char> = token.chars().collect();
    let all_cjk = !chars.is_empty() && chars.iter().all(|c| is_cjk(*c));

    if all_cjk && chars.len() > 1 {
        let segments = segment_cjk(token);
        let any_multi = segments.iter().any(|segment| segment.chars().count() > 1);
        if any_multi {
            for segment in &segments {
                if segment.chars().count() >= 2 && seen.insert(segment.to_lowercase()) {
                    tokens.push(segment.clone());
                }
            }
        } else if seen.insert(token.to_lowercase()) {
            tokens.push(token.to_string());
        }
    } else if seen.insert(token.to_lowercase()) {
        tokens.push(token.to_string());
    }
}

fn add_with_translations(token: &str, expanded: &mut Vec<String>, seen: &mut HashSet<String>) {
    if seen.insert(token.to_lowercase()) {
        expanded.push(token.to_string());
    }
    let lower = token.to_lowercase();
    if let Some(translations) = CJK_EN_TRANSLATIONS.get(lower.as_str()) {
        for translation in translations {
            if seen.insert(translation.to_lowercase()) {
                expanded.push((*translation).to_string());
            }
        }
    }
}
