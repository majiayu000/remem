use std::collections::HashSet;

use crate::memory::Memory;

pub(super) fn claim_terms(
    query_text: &str,
    core_terms: &[String],
    project: Option<&str>,
) -> Vec<String> {
    let entity_terms: HashSet<String> = crate::retrieval::entity::extract_entities("", query_text)
        .into_iter()
        .filter_map(|term| normalize_claim_token(&term))
        .collect();
    let project_terms: HashSet<String> = project
        .into_iter()
        .flat_map(|project| project.split(|c: char| !c.is_alphanumeric() && !is_cjk(c)))
        .filter_map(normalize_claim_token)
        .collect();

    core_terms
        .iter()
        .filter_map(|term| normalize_claim_token(term))
        .filter(|term| !entity_terms.contains(term) && !project_terms.contains(term))
        .collect()
}

pub(super) fn claim_term_coverage(memory: &Memory, claim_terms: &[String]) -> f64 {
    if claim_terms.is_empty() {
        return 1.0;
    }
    let haystack = format!("{} {}", memory.title, memory.text).to_lowercase();
    let matched = claim_terms
        .iter()
        .filter(|term| claim_term_matches(&haystack, term))
        .count();
    matched as f64 / claim_terms.len() as f64
}

fn claim_term_matches(haystack: &str, term: &str) -> bool {
    if haystack.contains(term) {
        return true;
    }
    if claim_term_aliases(term)
        .iter()
        .any(|alias| haystack.contains(alias))
    {
        return true;
    }
    claim_term_stems(term)
        .iter()
        .any(|stem| stem.chars().count() >= 3 && haystack.contains(stem.as_str()))
}

fn claim_term_aliases(term: &str) -> &'static [&'static str] {
    match term {
        "child" | "children" | "kid" | "kids" => &[
            "child",
            "children",
            "kid",
            "kids",
            "son",
            "daughter",
            "sons",
            "daughters",
        ],
        _ => &[],
    }
}

fn claim_term_stems(term: &str) -> Vec<String> {
    let mut stems = Vec::new();
    if let Some(stem) = term.strip_suffix("ing") {
        stems.push(stem.to_string());
    }
    if let Some(stem) = term.strip_suffix("ed") {
        stems.push(stem.to_string());
        stems.push(format!("{stem}e"));
    }
    if let Some(stem) = term.strip_suffix('s') {
        stems.push(stem.to_string());
    }
    stems
}

fn normalize_claim_token(term: &str) -> Option<String> {
    let normalized = term
        .trim_matches(|c: char| !c.is_alphanumeric() && !is_cjk(c))
        .to_lowercase();
    let min_len = if normalized.chars().any(is_cjk) { 2 } else { 3 };
    if normalized.chars().count() < min_len || is_generic_query_term(&normalized) {
        None
    } else {
        Some(normalized)
    }
}

fn is_generic_query_term(term: &str) -> bool {
    matches!(
        term,
        "all"
            | "and"
            | "are"
            | "did"
            | "does"
            | "for"
            | "from"
            | "current"
            | "had"
            | "has"
            | "have"
            | "handles"
            | "how"
            | "into"
            | "is"
            | "its"
            | "latest"
            | "onto"
            | "project"
            | "show"
            | "that"
            | "the"
            | "this"
            | "through"
            | "today"
            | "tomorrow"
            | "yesterday"
            | "before"
            | "after"
            | "during"
            | "only"
            | "production"
            | "was"
            | "were"
            | "what"
            | "when"
            | "where"
            | "which"
            | "who"
            | "why"
            | "with"
    )
}

fn is_cjk(c: char) -> bool {
    matches!(
        c,
        '\u{4E00}'..='\u{9FFF}' | '\u{3400}'..='\u{4DBF}' | '\u{F900}'..='\u{FAFF}'
    )
}
