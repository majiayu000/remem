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
    let subject = term[..*infix]
        .strip_suffix("是否")
        .unwrap_or(&term[..*infix]);
    let agent = &term[*infix + '由'.len_utf8()..];
    if subject.is_empty() || agent.is_empty() {
        return false;
    }
    let direct = format!("{agent}{predicate}{subject}");
    let responsible = (predicate != "负责").then(|| format!("{agent}负责{predicate}{subject}"));
    standalone_relation_matches(
        haystack,
        &direct,
        responsible.as_deref(),
        predicate,
        subject,
    )
}

fn standalone_relation_matches(
    haystack: &str,
    direct: &str,
    responsible: Option<&str>,
    predicate: &str,
    subject: &str,
) -> bool {
    let clauses = haystack
        .split(is_strong_clause_boundary)
        .map(str::trim)
        .filter(|clause| !clause.is_empty())
        .collect::<Vec<_>>();
    let matching = clauses
        .iter()
        .map(|clause| relation_clause_matches(clause, direct, responsible))
        .collect::<Vec<_>>();
    matching.iter().any(|matches| *matches)
        && clauses.iter().zip(matching).all(|(clause, matches)| {
            matches || !relation_clause_conflicts(clause, predicate, subject)
        })
}

fn relation_clause_matches(clause: &str, direct: &str, responsible: Option<&str>) -> bool {
    std::iter::once(direct).chain(responsible).any(|signature| {
        clause == signature
            || clause.strip_prefix(signature).is_some_and(|suffix| {
                suffix
                    .strip_prefix("，涉及")
                    .is_some_and(|detail| !detail.trim().is_empty())
            })
    })
}

fn relation_clause_conflicts(clause: &str, predicate: &str, subject: &str) -> bool {
    clause.contains(predicate) && clause.contains(subject)
}

fn is_strong_clause_boundary(character: char) -> bool {
    matches!(
        character,
        '。' | '！' | '？' | '；' | '.' | '!' | '?' | ';' | '\n' | '\r'
    )
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
