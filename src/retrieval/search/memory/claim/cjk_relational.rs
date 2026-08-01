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
const CJK_RELATION_NEGATIVE_INFIXES: &[&str] = &[
    "不再负责",
    "不负责",
    "拒绝负责",
    "尚未负责",
    "未负责",
    "没有负责",
    "停止负责",
    "不再",
    "并不",
    "不",
    "未",
    "没有",
    "拒绝",
    "停止",
];

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

enum RelationSegment<'a> {
    Assertion(RelationAssertion<'a>),
    Ignored,
    Unknown,
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
        let mut unknown_prefix = false;
        for segment in clause.split([',', '，']) {
            match parse_relation_segment(segment, predicate, subject) {
                RelationSegment::Assertion(RelationAssertion::Positive { agent, .. })
                    if !unknown_prefix && relation_agent_contains(agent, expected_agent) =>
                {
                    matches = true
                }
                RelationSegment::Assertion(RelationAssertion::Positive {
                    resets_state: true,
                    ..
                }) if !unknown_prefix => matches = false,
                RelationSegment::Assertion(RelationAssertion::Negative { agent })
                    if !unknown_prefix && relation_agent_contains(agent, expected_agent) =>
                {
                    matches = false;
                }
                RelationSegment::Unknown => unknown_prefix = true,
                _ => {}
            }
        }
    }
    matches
}

fn parse_relation_segment<'a>(
    segment: &'a str,
    predicate: &str,
    subject: &str,
) -> RelationSegment<'a> {
    let segment = segment.trim();
    if segment.is_empty() {
        return RelationSegment::Ignored;
    }
    if CJK_RELATION_HISTORICAL_CARRIERS
        .iter()
        .any(|carrier| segment.starts_with(carrier))
    {
        return RelationSegment::Ignored;
    }

    for suffix in CJK_RELATION_NEGATIVE_SUFFIXES {
        if let Some(statement) = segment.strip_suffix(suffix) {
            return parse_relation_agent(statement, predicate, subject)
                .map_or(RelationSegment::Unknown, |agent| {
                    RelationSegment::Assertion(RelationAssertion::Negative { agent })
                });
        }
    }
    for carrier in CJK_RELATION_NEGATIVE_CARRIERS {
        if let Some(statement) = segment.strip_prefix(carrier) {
            return parse_relation_agent(statement, predicate, subject)
                .map_or(RelationSegment::Unknown, |agent| {
                    RelationSegment::Assertion(RelationAssertion::Negative { agent })
                });
        }
    }
    for infix in CJK_RELATION_NEGATIVE_INFIXES {
        let negative_tail = format!("{infix}{predicate}{subject}");
        if let Some(agent) = segment
            .strip_suffix(&negative_tail)
            .filter(|agent| valid_relation_agent(agent))
        {
            return RelationSegment::Assertion(RelationAssertion::Negative { agent });
        }
    }

    for carrier in CJK_RELATION_POSITIVE_CARRIERS {
        if let Some(statement) = segment.strip_prefix(carrier) {
            return parse_relation_agent(statement, predicate, subject).map_or(
                RelationSegment::Unknown,
                |agent| {
                    RelationSegment::Assertion(RelationAssertion::Positive {
                        agent,
                        resets_state: true,
                    })
                },
            );
        }
    }
    if let Some(statement) = strip_relation_label(segment, predicate) {
        return parse_relation_agent(statement, predicate, subject).map_or(
            RelationSegment::Unknown,
            |agent| {
                RelationSegment::Assertion(RelationAssertion::Positive {
                    agent,
                    resets_state: true,
                })
            },
        );
    }
    parse_relation_agent(segment, predicate, subject).map_or(RelationSegment::Unknown, |agent| {
        RelationSegment::Assertion(RelationAssertion::Positive {
            agent,
            resets_state: false,
        })
    })
}

fn strip_relation_label<'a>(segment: &'a str, predicate: &str) -> Option<&'a str> {
    let (label, statement) = segment
        .split_once('：')
        .or_else(|| segment.split_once(':'))?;
    let label = label.trim();
    let predicate_label = format!("{predicate}者");
    let current_predicate_label = format!("当前{predicate}者");
    (label == predicate_label
        || label == current_predicate_label
        || (predicate == "负责" && matches!(label, "负责人" | "当前负责人")))
    .then_some(statement.trim())
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

fn relation_agent_contains(candidate: &str, expected: &str) -> bool {
    let candidate = candidate.trim();
    let Some(group) = candidate.strip_suffix("共同") else {
        return relation_agents_equal(candidate, expected);
    };
    let mut members = group.split(['和', '与', '、']);
    let mut found = false;
    for member in &mut members {
        if !valid_relation_agent(member) {
            return false;
        }
        found |= relation_agents_equal(member, expected);
    }
    found
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
        || character == '+'
        || character == '.'
        || character == '/'
        || character == '、'
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
