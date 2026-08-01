use super::is_cjk;

const LEXICAL_YOU_PREFIXES: &[char] = &[
    '自', '理', '事', '案', '缘', '原', '因', '来', '情', '根', '无', '何', '端', '所',
];
const LEXICAL_YOU_SUFFIXES: &[char] = &['于', '来', '衷', '此', '头'];
const CJK_RELATION_POSITIVE_CARRIERS: &[&str] = &[
    "现在由",
    "目前由",
    "当前由",
    "而是",
    "现由",
    "改由",
    "转由",
    "由",
];
const CJK_RELATION_NEGATIVE_CARRIERS: &[&str] = &[
    "不是现在由",
    "不是目前由",
    "不是当前由",
    "尚未让",
    "不允许",
    "拒绝让",
    "拒绝改由",
    "现在不由",
    "目前不由",
    "当前不由",
    "不再由",
    "不是",
    "非",
];
const CJK_RELATION_HISTORICAL_CARRIERS: &[&str] =
    &["过去", "曾由", "原由", "此前由", "原先由", "以前由"];
const CJK_RELATION_CONDITIONAL_CARRIERS: &[&str] = &["如果", "若", "要是", "假如", "假设", "倘若"];
const CJK_RELATION_NEGATIVE_SUFFIXES: &[&str] = &["的说法并不成立", "只是一个假设"];

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
    current_relation_matches(haystack, agent, predicate, subject)
}

#[derive(Clone, Copy)]
enum RelationAssertion<'a> {
    Positive { agent: &'a str, resets_state: bool },
    Negative { agent: &'a str },
}

fn current_relation_matches(
    haystack: &str,
    expected_agent: &str,
    predicate: &str,
    subject: &str,
) -> bool {
    let mut matches = false;
    for clause in haystack.split(is_strong_clause_boundary) {
        let clause = clause.trim();
        if clause.is_empty()
            || CJK_RELATION_CONDITIONAL_CARRIERS
                .iter()
                .any(|carrier| clause.starts_with(carrier))
        {
            continue;
        }
        for segment in clause.split([',', '，']) {
            match parse_relation_assertion(segment, predicate, subject) {
                Some(RelationAssertion::Positive { agent, .. })
                    if relation_agents_equal(agent, expected_agent) =>
                {
                    matches = true
                }
                Some(RelationAssertion::Positive {
                    resets_state: true, ..
                }) => matches = false,
                Some(RelationAssertion::Negative { agent })
                    if relation_agents_equal(agent, expected_agent) =>
                {
                    matches = false;
                }
                _ => {}
            }
        }
    }
    matches
}

fn parse_relation_assertion<'a>(
    segment: &'a str,
    predicate: &str,
    subject: &str,
) -> Option<RelationAssertion<'a>> {
    let segment = segment.trim();
    if segment.is_empty()
        || CJK_RELATION_HISTORICAL_CARRIERS
            .iter()
            .any(|carrier| segment.starts_with(carrier))
    {
        return None;
    }

    for suffix in CJK_RELATION_NEGATIVE_SUFFIXES {
        if let Some(statement) = segment.strip_suffix(suffix) {
            return parse_relation_agent(statement, predicate, subject)
                .map(|agent| RelationAssertion::Negative { agent });
        }
    }
    for carrier in CJK_RELATION_NEGATIVE_CARRIERS {
        if let Some(statement) = segment.strip_prefix(carrier) {
            return parse_relation_agent(statement, predicate, subject)
                .map(|agent| RelationAssertion::Negative { agent });
        }
    }
    if let Some((agent, statement)) = segment.split_once("不再负责") {
        if statement == format!("{predicate}{subject}") && valid_relation_agent(agent) {
            return Some(RelationAssertion::Negative { agent });
        }
    }
    if let Some((agent, statement)) = segment.split_once("不再") {
        if statement == format!("{predicate}{subject}") && valid_relation_agent(agent) {
            return Some(RelationAssertion::Negative { agent });
        }
    }

    for carrier in CJK_RELATION_POSITIVE_CARRIERS {
        if let Some(statement) = segment.strip_prefix(carrier) {
            return parse_relation_agent(statement, predicate, subject).map(|agent| {
                RelationAssertion::Positive {
                    agent,
                    resets_state: true,
                }
            });
        }
    }
    parse_relation_agent(segment, predicate, subject).map(|agent| RelationAssertion::Positive {
        agent,
        resets_state: false,
    })
}

fn parse_relation_agent<'a>(statement: &'a str, predicate: &str, subject: &str) -> Option<&'a str> {
    let responsible_tail = format!("负责{predicate}{subject}");
    let direct_tail = format!("{predicate}{subject}");
    if predicate != "负责" {
        if let Some(agent) = statement
            .strip_suffix(&responsible_tail)
            .filter(|agent| valid_relation_agent(agent))
        {
            return Some(agent);
        }
    }
    statement
        .strip_suffix(&direct_tail)
        .filter(|agent| valid_relation_agent(agent))
}

fn valid_relation_agent(agent: &str) -> bool {
    let agent = agent.trim();
    !agent.is_empty()
        && agent.chars().count() <= 32
        && agent.chars().all(is_relation_agent_character)
}

fn relation_agents_equal(left: &str, right: &str) -> bool {
    left.split_whitespace().eq(right.split_whitespace())
}

fn is_strong_clause_boundary(character: char) -> bool {
    matches!(
        character,
        '。' | '！' | '？' | '；' | '!' | '?' | ';' | '\n' | '\r'
    )
}

fn is_relation_agent_character(character: char) -> bool {
    is_cjk(character)
        || character.is_ascii_alphanumeric()
        || character == '_'
        || character == '-'
        || character.is_whitespace()
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
