use super::is_cjk;

const LEXICAL_YOU_PREFIXES: &[char] = &[
    '自', '理', '事', '案', '缘', '原', '因', '来', '情', '根', '无', '何', '端', '所',
];
const LEXICAL_YOU_SUFFIXES: &[char] = &['于', '来', '衷', '此', '头'];
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
    has_current_relation_signature(haystack, agent, predicate, subject, false)
        || (predicate != "负责"
            && has_current_relation_signature(haystack, agent, predicate, subject, true))
}

fn has_current_relation_signature(
    haystack: &str,
    agent: &str,
    predicate: &str,
    subject: &str,
    with_responsible_modifier: bool,
) -> bool {
    let signature = if with_responsible_modifier {
        format!("{agent}负责{predicate}{subject}")
    } else {
        format!("{agent}{predicate}{subject}")
    };
    haystack.match_indices(&signature).any(|(start, matched)| {
        is_positive_statement_start(haystack, start)
            && latest_explicit_state_agent(haystack, start + matched.len(), predicate, subject)
                .is_none_or(|current_agent| current_agent == agent)
    })
}

fn is_positive_statement_start(haystack: &str, signature_start: usize) -> bool {
    let prefix = &haystack[..signature_start];
    let boundary_start = prefix
        .char_indices()
        .rev()
        .find(|(_, character)| is_strong_clause_boundary(*character))
        .map_or(0, |(index, character)| index + character.len_utf8());
    let reset_start = CJK_RELATION_POSITIVE_RESETS
        .iter()
        .filter_map(|marker| {
            prefix[boundary_start..]
                .rmatch_indices(marker)
                .next()
                .map(|(index, _)| boundary_start + index + marker.len())
        })
        .max()
        .unwrap_or(boundary_start);
    let statement_prefix = prefix[reset_start..].trim_matches(is_soft_relation_separator);
    statement_prefix.is_empty() || statement_prefix == "由"
}

fn latest_explicit_state_agent<'a>(
    haystack: &'a str,
    after: usize,
    predicate: &str,
    subject: &str,
) -> Option<&'a str> {
    CJK_RELATION_POSITIVE_RESETS
        .iter()
        .flat_map(|marker| {
            haystack[after..]
                .match_indices(marker)
                .map(move |(index, _)| (after + index, *marker))
        })
        .filter_map(|(index, marker)| {
            let state_start = index + marker.len();
            let state = &haystack[state_start..];
            let state_end = state
                .char_indices()
                .find(|(_, character)| is_strong_clause_boundary(*character))
                .map_or(state.len(), |(end, _)| end);
            explicit_state_agent(&state[..state_end], predicate, subject)
                .map(|agent| (index, agent))
        })
        .max_by_key(|(index, _)| *index)
        .map(|(_, agent)| agent)
}

fn explicit_state_agent<'a>(state: &'a str, predicate: &str, subject: &str) -> Option<&'a str> {
    let state = state.trim_matches(is_soft_relation_separator);
    let direct_tail = format!("{predicate}{subject}");
    let responsible_tail = format!("负责{predicate}{subject}");
    let matched_agent = [responsible_tail.as_str(), direct_tail.as_str()]
        .into_iter()
        .filter_map(|tail| {
            state.find(tail).and_then(|tail_start| {
                let agent = state[..tail_start].trim_matches(is_soft_relation_separator);
                (!agent.is_empty()
                    && agent.chars().count() <= 32
                    && agent.chars().all(is_relation_agent_character))
                .then_some(agent)
            })
        })
        .next();
    matched_agent
}

fn is_strong_clause_boundary(character: char) -> bool {
    matches!(
        character,
        '。' | '！' | '？' | '；' | '!' | '?' | ';' | '\n' | '\r'
    )
}

fn is_soft_relation_separator(character: char) -> bool {
    character.is_whitespace() || matches!(character, ',' | '，' | ':' | '：' | '-' | '—' | '–')
}

fn is_relation_agent_character(character: char) -> bool {
    is_cjk(character) || character.is_ascii_alphanumeric() || character == '_'
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
