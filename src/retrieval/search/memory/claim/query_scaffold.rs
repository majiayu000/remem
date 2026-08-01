use std::ops::Range;

pub(super) fn strip_conversational_prefix<'a>(
    query: &'a str,
    explicit_entity_terms: &[String],
) -> &'a str {
    let trimmed = query.trim_start();
    let spans = first_query_word_spans(trimmed, 4);
    if spans.len() != 4 {
        return query;
    }
    if !has_conversational_word_gaps(trimmed, &spans) {
        return query;
    }

    let word = |position: usize| &trimmed[spans[position].clone()];
    if !matches!(word(0).to_ascii_lowercase().as_str(), "please" | "kindly")
        || !word(1).eq_ignore_ascii_case("tell")
        || !word(2).eq_ignore_ascii_case("me")
    {
        return query;
    }

    let fourth = word(3);
    let fourth_lower = fourth.to_ascii_lowercase();
    if ["if", "whether", "about"].contains(&fourth_lower.as_str()) {
        return &trimmed[spans[3].end..];
    }
    if [
        "who", "what", "when", "where", "why", "how", "which", "the", "a", "an",
    ]
    .contains(&fourth_lower.as_str())
        || explicit_entity_terms
            .iter()
            .any(|entity| first_entity_word(entity).eq_ignore_ascii_case(fourth))
    {
        return &trimmed[spans[2].end..];
    }
    query
}

fn has_conversational_word_gaps(query: &str, spans: &[Range<usize>]) -> bool {
    spans.windows(2).all(|pair| {
        let gap = &query[pair[0].end..pair[1].start];
        !gap.is_empty()
            && gap.chars().all(|character| {
                character.is_whitespace() || matches!(character, ',' | '，' | '—' | '–')
            })
    })
}

fn first_query_word_spans(query: &str, limit: usize) -> Vec<Range<usize>> {
    let mut spans = Vec::new();
    let mut index = 0;
    for _ in 0..limit {
        if !spans.is_empty() {
            index += query[index..]
                .find(is_query_word_character)
                .unwrap_or(query.len() - index);
        }
        let start = index;
        index += query[index..]
            .find(|character: char| !is_query_word_character(character))
            .unwrap_or(query.len() - index);
        if start == index {
            break;
        }
        spans.push(start..index);
    }
    spans
}

fn is_query_word_character(character: char) -> bool {
    character.is_alphanumeric() || character == '_'
}

fn first_entity_word(entity: &str) -> &str {
    entity
        .split(|character: char| !character.is_alphanumeric() && character != '_')
        .find(|word| !word.is_empty())
        .unwrap_or("")
}
