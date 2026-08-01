use super::is_cjk;

const LEXICAL_YOU_PREFIXES: &[char] = &[
    '自', '理', '事', '案', '缘', '原', '因', '来', '情', '根', '无', '何', '端', '所',
];
const LEXICAL_YOU_SUFFIXES: &[char] = &['于', '来', '衷', '此', '头'];

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
    let subject = &term[..*infix];
    let agent = &term[*infix + '由'.len_utf8()..];
    if subject.is_empty() || agent.is_empty() {
        return false;
    }
    haystack.contains(&format!("{agent}{predicate}{subject}"))
}

fn has_lexical_you_suffix(term: &str, infix: usize) -> bool {
    term[infix + '由'.len_utf8()..]
        .chars()
        .next()
        .is_some_and(|suffix| LEXICAL_YOU_SUFFIXES.contains(&suffix))
}

fn has_lexical_you_prefix(term: &str, infix: usize) -> bool {
    term[..infix]
        .chars()
        .next_back()
        .is_some_and(|prefix| LEXICAL_YOU_PREFIXES.contains(&prefix))
}
