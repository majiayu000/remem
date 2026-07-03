use std::collections::BTreeSet;

use anyhow::{anyhow, Result};

use super::run::corpus_memory_matches_query_filter;
use super::types::{contains_case_insensitive, GoldenQuery};

const ASSOCIATIVE_ENTITY_TYPES: &[&str] =
    &["file_path", "crate", "error_signature", "issue_number"];

pub(in crate::eval) fn validate_associative_query(
    query: &GoldenQuery,
    fixture_memories: &[crate::memory::Memory],
) -> Result<()> {
    let hop_path = query.hop_path.as_ref().ok_or_else(|| {
        anyhow!(
            "golden eval associative query {} must declare hop_path",
            query.id
        )
    })?;
    validate_hop_path_field(&query.id, "source", &hop_path.source)?;
    validate_hop_path_field(&query.id, "entity_type", &hop_path.entity_type)?;
    validate_hop_path_field(&query.id, "entity", &hop_path.entity)?;
    validate_hop_path_field(&query.id, "target", &hop_path.target)?;
    if !ASSOCIATIVE_ENTITY_TYPES.contains(&hop_path.entity_type.as_str()) {
        return Err(anyhow!(
            "golden eval associative query {} hop_path entity_type {} must be one of {}",
            query.id,
            hop_path.entity_type,
            ASSOCIATIVE_ENTITY_TYPES.join(", ")
        ));
    }

    let source = find_hop_memory(query, fixture_memories, &hop_path.source).ok_or_else(|| {
        anyhow!(
            "golden eval associative query {} hop_path source {} is not backed by fixture corpus",
            query.id,
            hop_path.source
        )
    })?;
    let target = find_hop_memory(query, fixture_memories, &hop_path.target).ok_or_else(|| {
        anyhow!(
            "golden eval associative query {} hop_path target {} is not backed by fixture corpus",
            query.id,
            hop_path.target
        )
    })?;
    if !memory_contains(source, &hop_path.entity) {
        return Err(anyhow!(
            "golden eval associative query {} hop_path source {} does not contain entity {}",
            query.id,
            hop_path.source,
            hop_path.entity
        ));
    }
    if !memory_contains(target, &hop_path.entity) {
        return Err(anyhow!(
            "golden eval associative query {} hop_path target {} does not contain entity {}",
            query.id,
            hop_path.target,
            hop_path.entity
        ));
    }
    if !query
        .expected_refs()
        .iter()
        .any(|expected_ref| expected_ref.matches(target))
    {
        return Err(anyhow!(
            "golden eval associative query {} hop_path target {} must be covered by expected evidence",
            query.id,
            hop_path.target
        ));
    }

    let overlap = query_target_shared_tokens(&query.query, &target.text);
    if !overlap.is_empty() {
        return Err(anyhow!(
            "golden eval associative query {} leaks query/target token overlap: {}",
            query.id,
            overlap.into_iter().collect::<Vec<_>>().join(", ")
        ));
    }
    Ok(())
}

fn validate_hop_path_field(query_id: &str, field: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() {
        return Err(anyhow!(
            "golden eval associative query {query_id} hop_path {field} must not be empty"
        ));
    }
    Ok(())
}

fn find_hop_memory<'a>(
    query: &GoldenQuery,
    fixture_memories: &'a [crate::memory::Memory],
    topic_key: &str,
) -> Option<&'a crate::memory::Memory> {
    fixture_memories.iter().find(|memory| {
        corpus_memory_matches_query_filter(memory, query)
            && memory.topic_key.as_deref() == Some(topic_key)
    })
}

fn memory_contains(memory: &crate::memory::Memory, needle: &str) -> bool {
    contains_case_insensitive(&memory.title, needle)
        || contains_case_insensitive(&memory.text, needle)
}

pub(in crate::eval) fn query_target_shared_tokens(
    query: &str,
    target_text: &str,
) -> BTreeSet<String> {
    let query_tokens = normalized_text_tokens(query);
    let target_tokens = normalized_text_tokens(target_text);
    query_tokens
        .intersection(&target_tokens)
        .cloned()
        .collect::<BTreeSet<_>>()
}

pub(in crate::eval) fn normalized_text_tokens(value: &str) -> BTreeSet<String> {
    value
        .split(|character: char| !character.is_ascii_alphanumeric())
        .filter_map(|token| {
            let token = token.trim().to_ascii_lowercase();
            (token.len() > 2).then_some(token)
        })
        .collect()
}
