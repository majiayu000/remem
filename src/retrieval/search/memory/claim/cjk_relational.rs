use super::is_cjk;

pub(super) fn relational_term_matches(haystack: &str, term: &str, predicate: &str) -> bool {
    if !term.chars().all(is_cjk) || !predicate.chars().all(is_cjk) {
        return false;
    }
    let Some(infix) = term.rfind('由') else {
        return false;
    };
    let subject = &term[..infix];
    let agent = &term[infix + '由'.len_utf8()..];
    if subject.is_empty() || agent.is_empty() {
        return false;
    }
    haystack.contains(&format!("{agent}{predicate}{subject}"))
}
