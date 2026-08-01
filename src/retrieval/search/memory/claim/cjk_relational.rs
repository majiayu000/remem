use super::is_cjk;

const LEXICAL_YOU_PREFIXES: &[char] = &[
    '自', '理', '事', '案', '缘', '原', '因', '来', '情', '根', '无', '何', '端', '所',
];
const LEXICAL_YOU_SUFFIXES: &[char] = &['于', '来', '衷', '此', '头'];
const CJK_RELATION_HYPOTHETICAL_PREFIXES: &[&str] = &["如果", "假如", "假设", "倘若"];
const CJK_RELATION_NON_CURRENT_PREFIXES: &[&str] = &[
    "不是", "并非", "不再", "没有", "拒绝", "禁止", "避免", "过去", "曾经", "此前", "原先", "以前",
    "一度",
];
const CJK_RELATION_POSITIVE_RESETS: &[&str] =
    &["而是", "现在由", "目前由", "当前由", "现由", "改由", "转由"];

pub(super) fn relational_term_matches(haystack: &str, term: &str, predicate: &str) -> bool {
    if !term.chars().all(is_cjk) || !predicate.chars().all(is_cjk) {
        return false;
    }
    let markers = term
        .match_indices('由')
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    let candidates = markers
        .iter()
        .copied()
        .filter(|index| {
            !has_lexical_you_suffix(term, *index) && !has_lexical_you_prefix(term, *index)
        })
        .collect::<Vec<_>>();
    let [infix] = candidates.as_slice() else {
        return false;
    };
    let subject = term[..*infix]
        .strip_suffix("是否")
        .unwrap_or(&term[..*infix]);
    let agent = &term[*infix + '由'.len_utf8()..];
    if subject.is_empty() || agent.is_empty() {
        return false;
    }
    has_positive_relation_signature(haystack, &format!("{agent}{predicate}{subject}"))
        || (predicate != "负责"
            && has_positive_relation_signature(
                haystack,
                &format!("{agent}负责{predicate}{subject}"),
            ))
}

fn has_positive_relation_signature(haystack: &str, signature: &str) -> bool {
    haystack
        .match_indices(signature)
        .any(|(start, _)| has_positive_relation_context(haystack, start))
}

fn has_positive_relation_context(haystack: &str, signature_start: usize) -> bool {
    let prefix = &haystack[..signature_start];
    let clause_start = prefix
        .char_indices()
        .rev()
        .find(|(_, character)| {
            matches!(
                character,
                '。' | '！' | '？' | '；' | '!' | '?' | ';' | '\n' | '\r'
            )
        })
        .map_or(0, |(index, character)| index + character.len_utf8());
    let clause_prefix = &prefix[clause_start..];
    if CJK_RELATION_HYPOTHETICAL_PREFIXES
        .iter()
        .any(|marker| clause_prefix.contains(marker))
    {
        return false;
    }
    let state_start = CJK_RELATION_POSITIVE_RESETS
        .iter()
        .filter_map(|marker| {
            clause_prefix
                .rmatch_indices(marker)
                .next()
                .map(|(index, _)| index + marker.len())
        })
        .max()
        .unwrap_or(0);
    !CJK_RELATION_NON_CURRENT_PREFIXES
        .iter()
        .any(|marker| clause_prefix[state_start..].contains(marker))
}

fn has_lexical_you_suffix(term: &str, infix: usize) -> bool {
    term[infix + '由'.len_utf8()..]
        .chars()
        .next()
        .is_some_and(|suffix| LEXICAL_YOU_SUFFIXES.contains(&suffix))
}

fn has_lexical_you_prefix(term: &str, infix: usize) -> bool {
    let subject = &term[..infix];
    if subject.ends_with("根因") {
        return false;
    }
    subject
        .chars()
        .next_back()
        .is_some_and(|prefix| LEXICAL_YOU_PREFIXES.contains(&prefix))
}
