use super::is_cjk;

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
        .filter(|index| !is_youyu(term, *index) && !is_lexical_you_compound(term, *index))
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

fn is_youyu(term: &str, infix: usize) -> bool {
    term[infix + '由'.len_utf8()..].starts_with('于')
}

fn is_lexical_you_compound(term: &str, infix: usize) -> bool {
    term[..infix].chars().next_back().is_some_and(|prefix| {
        matches!(
            prefix,
            '自' | '理' | '事' | '案' | '缘' | '原' | '因' | '来' | '情'
        )
    })
}
