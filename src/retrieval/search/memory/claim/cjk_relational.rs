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
    markers
        .into_iter()
        .filter(|infix| !has_lexical_you_suffix(term, *infix))
        .filter(|infix| !has_lexical_you_context(term, *infix))
        .filter(|infix| {
            let subject = term[..*infix]
                .strip_suffix("是否")
                .unwrap_or(&term[..*infix]);
            let agent = &term[*infix + '由'.len_utf8()..];
            if subject.is_empty()
                || agent.is_empty()
                || agent.starts_with('由')
                || subject.ends_with(['转', '改'])
                || agent.contains("转由")
                || agent.contains("改由")
            {
                return false;
            }
            let direct = format!("{agent}{predicate}{subject}");
            let responsible =
                (predicate != "负责").then(|| format!("{agent}负责{predicate}{subject}"));
            standalone_relation_matches(
                haystack,
                &direct,
                responsible.as_deref(),
                predicate,
                subject,
            )
        })
        .take(2)
        .count()
        == 1
}

fn standalone_relation_matches(
    haystack: &str,
    direct: &str,
    responsible: Option<&str>,
    predicate: &str,
    subject: &str,
) -> bool {
    let clauses = strong_relation_clauses(haystack)
        .into_iter()
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

fn strong_relation_clauses(text: &str) -> Vec<&str> {
    let mut clauses = Vec::new();
    let mut start = 0;
    let mut previous = None;
    for (index, character) in text.char_indices() {
        let next = text[index + character.len_utf8()..].chars().next();
        let identifier_period = character == '.'
            && previous.is_some_and(|value: char| value.is_ascii_alphanumeric())
            && next.is_some_and(|value| value.is_ascii_alphanumeric());
        if !identifier_period
            && matches!(
                character,
                '。' | '！' | '？' | '；' | '．' | '.' | '!' | '?' | ';' | '\n' | '\r'
            )
        {
            clauses.push(&text[start..index]);
            start = index + character.len_utf8();
        }
        previous = Some(character);
    }
    clauses.push(&text[start..]);
    clauses
}

fn has_lexical_you_suffix(term: &str, infix: usize) -> bool {
    term[infix + '由'.len_utf8()..]
        .chars()
        .next()
        .is_some_and(|suffix| LEXICAL_YOU_SUFFIXES.contains(&suffix))
}

fn has_lexical_you_context(term: &str, infix: usize) -> bool {
    let subject = &term[..infix];
    let Some(prefix) = subject.chars().next_back() else {
        return false;
    };
    if !LEXICAL_YOU_PREFIXES.contains(&prefix) {
        return false;
    }
    let before_prefix = subject[..subject.len() - prefix.len_utf8()]
        .chars()
        .next_back();
    let after_marker = term[infix + '由'.len_utf8()..].chars().next();
    subject.chars().count() == 1 || before_prefix == Some('由') || after_marker == Some('由')
}
