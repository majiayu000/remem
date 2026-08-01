use crate::retrieval::query_expand::core_tokens;

use super::is_cjk;

const LEXICAL_YOU_PREFIXES: &[char] = &['自', '理', '缘', '原'];

pub(super) fn relational_core_tokens(query: &str) -> Option<Vec<String>> {
    let clean = query.trim_matches(|character| !is_cjk(character));
    let characters = clean.chars().collect::<Vec<_>>();
    if characters.len() < 7 || !characters.iter().all(|character| is_cjk(*character)) {
        return None;
    }

    let infix = (1..characters.len() - 1).rev().find(|index| {
        characters[*index] == '由'
            && !LEXICAL_YOU_PREFIXES.contains(&characters[index.saturating_sub(1)])
    })?;
    if infix < 2 || characters.len() - infix - 1 < 4 {
        return None;
    }

    let subject = characters[..infix].iter().collect::<String>();
    let agent_and_predicate = characters[infix + 1..].iter().collect::<String>();
    let mut tokens = core_tokens(&subject);
    let mut tail_tokens = core_tokens(&agent_and_predicate);
    if tail_tokens.len() == 1 {
        tail_tokens = split_unknown_agent_and_predicate(&tail_tokens[0])?;
    }
    tokens.extend(tail_tokens);
    Some(tokens)
}

fn split_unknown_agent_and_predicate(token: &str) -> Option<Vec<String>> {
    let characters = token.chars().collect::<Vec<_>>();
    let split = characters.len().checked_sub(2)?;
    if split < 2 {
        return None;
    }
    Some(vec![
        characters[..split].iter().collect(),
        characters[split..].iter().collect(),
    ])
}
