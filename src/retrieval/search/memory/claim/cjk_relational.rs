use super::is_cjk;

pub(super) fn relational_term_matches(haystack: &str, term: &str) -> bool {
    if !term.chars().all(is_cjk) {
        return false;
    }
    let Some(infix) = term.rfind('由') else {
        return false;
    };
    let subject = &term[..infix];
    let agent = &term[infix + '由'.len_utf8()..];
    !subject.is_empty()
        && !agent.is_empty()
        && haystack.contains(subject)
        && haystack.contains(agent)
}
