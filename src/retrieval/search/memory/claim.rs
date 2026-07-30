use std::collections::HashSet;

use anyhow::Result;
use rusqlite::Connection;

#[cfg(test)]
use crate::memory::Memory;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) enum RelationKind {
    Affect,
    Block,
    Delete,
    Fix,
    Maintain,
    Own,
    Supersede,
    Use,
    Verify,
}

pub(super) fn entity_scope_candidates(query_text: &str, project: Option<&str>) -> Vec<String> {
    let project_terms = project_entity_terms(project);
    crate::retrieval::entity::extract_entities("", query_text)
        .into_iter()
        .filter(|term| {
            normalize_claim_token(term).is_some_and(|normalized| {
                !project_terms.contains(&normalized) && text_contains_exact_token(query_text, term)
            })
        })
        .collect()
}

pub(super) fn text_contains_exact_token(text: &str, term: &str) -> bool {
    if term.chars().any(is_cjk) {
        return text_contains_cjk_term(text, term);
    }
    text_contains_phrase_boundary(text, term)
}

pub(super) fn claim_terms(
    core_terms: &[String],
    project: Option<&str>,
    explicit_entity_terms: &[String],
) -> Vec<String> {
    let entity_terms: HashSet<String> = explicit_entity_terms
        .iter()
        .filter_map(|term| normalize_claim_token(term))
        .collect();
    let project_terms = project_entity_terms(project);

    core_terms
        .iter()
        .filter_map(|term| normalize_claim_token(term))
        .filter(|term| !entity_terms.contains(term) && !project_terms.contains(term))
        .collect()
}

#[cfg(test)]
pub(super) fn claim_term_coverage(memory: &Memory, claim_terms: &[String]) -> f64 {
    claim_text_coverage(&format!("{} {}", memory.title, memory.text), claim_terms)
}

pub(super) fn claim_text_coverage(text: &str, claim_terms: &[String]) -> f64 {
    if claim_terms.is_empty() {
        return 1.0;
    }
    claim_text_match_count(text, claim_terms) as f64 / claim_terms.len() as f64
}

pub(super) fn claim_text_match_count(text: &str, claim_terms: &[String]) -> usize {
    let haystack = text.to_lowercase();
    claim_terms
        .iter()
        .filter(|term| claim_term_matches(&haystack, term))
        .count()
}

fn claim_term_matches(haystack: &str, term: &str) -> bool {
    if term.chars().any(is_cjk) {
        return text_contains_cjk_term(haystack, term);
    }
    if text_contains_phrase_boundary(haystack, term) {
        return true;
    }
    let aliases = claim_term_aliases(term);
    if aliases
        .iter()
        .any(|alias| text_contains_phrase_boundary(haystack, alias))
    {
        return true;
    }
    std::iter::once(term)
        .chain(aliases.iter().copied())
        .filter(|candidate| {
            candidate
                .chars()
                .all(|character| character.is_alphanumeric())
        })
        .any(|candidate| {
            haystack
                .split(|character: char| !character.is_alphanumeric() && !is_cjk(character))
                .filter(|word| !word.is_empty())
                .any(|word| claim_words_share_form(candidate, word))
        })
}

fn text_contains_cjk_term(text: &str, term: &str) -> bool {
    let needle = term.trim().to_lowercase();
    needle.chars().count() >= 2 && text.to_lowercase().contains(&needle)
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
        "host" | "hosted" | "hosting" => &[
            "host",
            "hosts",
            "hosted",
            "hosting",
            "runs on",
            "deployed on",
        ],
        "handle" | "handled" | "handler" | "handlers" | "handles" | "handling" | "responsible"
        | "responsibility" => &[
            "handle",
            "handled",
            "handler",
            "handlers",
            "handles",
            "handling",
            "responsible",
            "responsibility",
            "owner",
            "owners",
            "owned",
            "owning",
            "owns",
            "pager",
            "pagers",
        ],
        "own" | "owned" | "owner" | "owning" | "owns" => {
            &["own", "owned", "owner", "owners", "owning", "owns"]
        }
        "migrate" | "migrated" | "migration" => {
            &["migrate", "migrated", "migrates", "migrating", "migration"]
        }
        _ => &[],
    }
}

pub(super) fn is_relation_only_claim_term(term: &str) -> bool {
    let normalized = term
        .trim()
        .trim_matches(|character: char| !character.is_alphanumeric())
        .to_lowercase();
    if normalized.is_empty() || !normalized.chars().all(char::is_alphanumeric) {
        return false;
    }
    if normalized.chars().any(is_cjk) {
        return matches!(
            normalized.as_str(),
            "修复"
                | "删除"
                | "拥有"
                | "替代"
                | "使用"
                | "维护"
                | "影响"
                | "验证"
                | "负责"
                | "阻塞"
        );
    }
    expressed_relation_kinds(&normalized).len() == 1
}

pub(super) fn expressed_relation_kinds(text: &str) -> HashSet<RelationKind> {
    let mut relations = HashSet::new();
    for (relation, english_terms, cjk_terms) in [
        (RelationKind::Fix, &["fix", "repair"][..], &["修复"][..]),
        (
            RelationKind::Verify,
            &[
                "approve",
                "okayed",
                "sign",
                "signer",
                "verification",
                "verifier",
                "verify",
            ][..],
            &["验证"][..],
        ),
        (
            RelationKind::Supersede,
            &["replace", "supersede"][..],
            &["替代"][..],
        ),
        (RelationKind::Block, &["block"][..], &["阻塞"][..]),
        (RelationKind::Use, &["use"][..], &["使用"][..]),
        (RelationKind::Affect, &["affect"][..], &["影响"][..]),
        (
            RelationKind::Delete,
            &["delete", "remove"][..],
            &["删除"][..],
        ),
        (RelationKind::Maintain, &["maintain"][..], &["维护"][..]),
        (
            RelationKind::Own,
            &["own", "responsibility", "responsible"][..],
            &["负责", "拥有"][..],
        ),
    ] {
        let english_match = english_terms
            .iter()
            .any(|term| claim_text_match_count(text, &[(*term).to_string()]) > 0);
        if english_match || cjk_terms.iter().any(|term| text.contains(term)) {
            relations.insert(relation);
        }
    }
    relations
}

fn text_contains_phrase_boundary(text: &str, phrase: &str) -> bool {
    let haystack = text.to_lowercase();
    let needle = phrase.trim().to_lowercase();
    if needle.is_empty() {
        return false;
    }
    haystack.match_indices(&needle).any(|(start, _)| {
        let end = start + needle.len();
        let before = haystack[..start].chars().next_back();
        let after = haystack[end..].chars().next();
        before.is_none_or(|character| !is_claim_word_character(character))
            && after.is_none_or(|character| !is_claim_word_character(character))
    })
}

fn is_claim_word_character(character: char) -> bool {
    character.is_alphanumeric() || character == '-' || character == '_'
}

fn claim_words_share_form(left: &str, right: &str) -> bool {
    let left_forms = claim_word_forms(left);
    claim_word_forms(right)
        .iter()
        .any(|form| left_forms.contains(form))
}

fn claim_word_forms(word: &str) -> HashSet<String> {
    let word = word.to_lowercase();
    let mut forms = HashSet::from([word.clone()]);
    for suffix in ["ment", "ence", "ance", "ing"] {
        if let Some(stem) = word.strip_suffix(suffix) {
            insert_claim_form(&mut forms, stem);
        }
    }
    if let Some(stem) = word.strip_suffix("ied") {
        insert_claim_form(&mut forms, &format!("{stem}y"));
    }
    if let Some(stem) = word.strip_suffix("ed") {
        insert_claim_form(&mut forms, stem);
        insert_claim_form(&mut forms, &format!("{stem}e"));
    }
    if let Some(stem) = word.strip_suffix("ies") {
        insert_claim_form(&mut forms, &format!("{stem}y"));
    }
    if let Some(stem) = word.strip_suffix('s') {
        insert_claim_form(&mut forms, stem);
    }
    forms
}

fn insert_claim_form(forms: &mut HashSet<String>, form: &str) {
    if form.chars().count() >= 3 {
        forms.insert(form.to_string());
    }
}

fn normalize_claim_token(term: &str) -> Option<String> {
    let clean = term.trim_matches(|c: char| !c.is_alphanumeric() && !is_cjk(c));
    let short_ascii_identifier = {
        let length = clean.chars().count();
        length == 2
            && clean
                .chars()
                .all(|character| character.is_ascii_alphanumeric())
            && clean
                .chars()
                .any(|character| character.is_ascii_uppercase() || character.is_ascii_digit())
    };
    let normalized = clean.to_lowercase();
    let min_len = if normalized.chars().any(is_cjk) || short_ascii_identifier {
        2
    } else {
        3
    };
    if normalized.chars().count() < min_len
        || is_nonsemantic_claim_modifier(&normalized)
        || is_generic_query_term(&normalized)
    {
        None
    } else {
        Some(normalized)
    }
}

pub(super) fn is_nonsemantic_claim_modifier(term: &str) -> bool {
    matches!(
        term.trim().to_lowercase().as_str(),
        "current" | "currently" | "recent" | "recently" | "当前" | "目前" | "最近"
    )
}

pub(super) fn project_entity_terms(project: Option<&str>) -> HashSet<String> {
    project
        .into_iter()
        .flat_map(|project| project.split(|c: char| !c.is_alphanumeric() && !is_cjk(c)))
        .filter_map(normalize_claim_token)
        .collect()
}

pub(super) fn has_distinctive_entity_shape(term: &str) -> bool {
    let clean = term.trim_matches(|character: char| {
        !character.is_alphanumeric() && character != '-' && character != '_'
    });
    let mut characters = clean.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    let remaining = characters.collect::<Vec<_>>();
    let internal_uppercase = remaining.iter().any(|character| character.is_uppercase());
    let has_lowercase = clean.chars().any(|character| character.is_lowercase());
    let structural_marker = clean
        .chars()
        .any(|character| character.is_ascii_digit() || matches!(character, '-' | '_'));

    structural_marker || (first.is_uppercase() && internal_uppercase && has_lowercase)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn select_entity_anchors(
    conn: &Connection,
    candidates: &[String],
    project: Option<&str>,
    memory_type: Option<&str>,
    branch: Option<&str>,
    limit: i64,
    include_inactive: bool,
    include_suppressed: bool,
) -> Result<(Vec<String>, Vec<i64>)> {
    let mut lookups = Vec::with_capacity(candidates.len());
    for (index, term) in candidates.iter().enumerate() {
        let ids = crate::retrieval::entity::search_exact_entity_names_filtered(
            conn,
            std::slice::from_ref(term),
            project,
            memory_type,
            branch,
            limit,
            include_inactive,
        )?;
        let ids = super::suppression_filter::ids(conn, ids, include_suppressed)?;
        lookups.push((index, term, ids));
    }

    let distinctive = lookups
        .iter()
        .filter(|(_, term, _)| has_distinctive_entity_shape(term))
        .collect::<Vec<_>>();
    let selected = if distinctive.is_empty() {
        lookups
            .iter()
            .filter(|(_, term, ids)| !ids.is_empty() && !is_relation_only_claim_term(term))
            .min_by(|left, right| {
                left.2
                    .len()
                    .cmp(&right.2.len())
                    .then_with(|| right.0.cmp(&left.0))
            })
            .into_iter()
            .collect::<Vec<_>>()
    } else {
        distinctive
    };
    let terms = selected
        .iter()
        .map(|(_, term, _)| (*term).clone())
        .collect::<Vec<_>>();
    let mut matching_ids = HashSet::new();
    if selected.iter().all(|(_, _, ids)| !ids.is_empty()) {
        matching_ids.extend(selected.iter().flat_map(|(_, _, ids)| ids.iter().copied()));
    }
    let mut matching_ids = matching_ids.into_iter().collect::<Vec<_>>();
    matching_ids.sort_unstable();

    Ok((terms, matching_ids))
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
            | "何时"
            | "什么"
            | "什么时候"
            | "为何"
            | "为什么"
            | "哪里"
            | "哪些"
            | "哪个"
            | "如何"
            | "怎么"
            | "谁"
    )
}

pub(super) fn is_cjk(c: char) -> bool {
    matches!(
        c,
        '\u{4E00}'..='\u{9FFF}' | '\u{3400}'..='\u{4DBF}' | '\u{F900}'..='\u{FAFF}'
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entity_scope_candidates_exclude_project_and_generic_query_words() {
        let terms = entity_scope_candidates(
            "Has Project KestrelNook migrated NebulaLatch to Oracle Cloud?",
            Some("synthetic/kestrelnook"),
        );

        assert!(terms.iter().any(|term| term == "NebulaLatch"));
        assert!(!terms.iter().any(|term| term == "KestrelNook"));
        assert!(!terms.iter().any(|term| term == "Project"));
        assert!(!terms.iter().any(|term| term == "Has"));
    }

    #[test]
    fn entity_scope_candidates_reject_technical_substring_matches() {
        let terms =
            entity_scope_candidates("How can I remember capitalization rules?", Some("/repo"));

        assert!(!terms.iter().any(|term| term.eq_ignore_ascii_case("remem")));
        assert!(!terms.iter().any(|term| term.eq_ignore_ascii_case("api")));
    }

    #[test]
    fn known_entity_hosting_paraphrase_has_claim_support() {
        let memory = Memory {
            id: 1,
            session_id: None,
            project: "/repo".to_string(),
            topic_key: None,
            title: "NebulaLatch deployment".to_string(),
            text: "NebulaLatch runs on Oracle Cloud.".to_string(),
            memory_type: "decision".to_string(),
            files: None,
            created_at_epoch: 1,
            updated_at_epoch: 1,
            status: "active".to_string(),
            branch: None,
            scope: "project".to_string(),
        };
        let core_terms =
            crate::retrieval::query_expand::core_tokens("Where is NebulaLatch hosted?");
        let terms = claim_terms(&core_terms, Some("/repo"), &["NebulaLatch".to_string()]);

        assert_eq!(terms, vec!["hosted"]);
        assert_eq!(claim_term_coverage(&memory, &terms), 1.0);
    }

    #[test]
    fn unselected_title_case_candidates_remain_claim_terms() {
        let query = "Which Pager Handles NebulaLatch Through Its Owning Team?";
        let candidates = entity_scope_candidates(query, Some("/repo"));
        let core_terms = crate::retrieval::query_expand::core_tokens(query);
        let claims = claim_terms(&core_terms, Some("/repo"), &["NebulaLatch".to_string()]);

        assert!(candidates.iter().any(|term| term == "NebulaLatch"));
        for predicate in ["pager", "handles", "owning", "team"] {
            assert!(claims.iter().any(|term| term == predicate), "{claims:?}");
        }
    }

    #[test]
    fn arbitrary_title_case_predicate_is_not_removed_by_a_static_list() {
        let query = "Who Maintains NebulaLatch?";
        let core_terms = crate::retrieval::query_expand::core_tokens(query);
        let claims = claim_terms(&core_terms, Some("/repo"), &["NebulaLatch".to_string()]);

        assert_eq!(claims, vec!["maintains"]);
    }

    #[test]
    fn short_uppercase_qualifiers_remain_claim_terms() {
        let query = "Who verified HarborMint in EU with R2?";
        let core_terms = crate::retrieval::query_expand::core_tokens(query);
        let claims = claim_terms(&core_terms, Some("/repo"), &["HarborMint".to_string()]);

        assert!(claims.contains(&"eu".to_string()), "{claims:?}");
        assert!(claims.contains(&"r2".to_string()), "{claims:?}");
    }

    #[test]
    fn lowercase_short_query_words_are_not_claim_terms() {
        let core_terms =
            crate::retrieval::query_expand::core_tokens("Who is HarborMint assigned to?");
        let claims = claim_terms(&core_terms, Some("/repo"), &["HarborMint".to_string()]);

        assert!(!claims.contains(&"is".to_string()), "{claims:?}");
        assert!(!claims.contains(&"to".to_string()), "{claims:?}");
    }

    #[test]
    fn distinctive_scope_shape_rejects_plain_title_case_words() {
        assert!(has_distinctive_entity_shape("NebulaLatch"));
        assert!(has_distinctive_entity_shape("incident-17"));
        assert!(!has_distinctive_entity_shape("Maintains"));
        assert!(!has_distinctive_entity_shape("Current"));
    }

    #[test]
    fn fallback_anchor_excludes_indexed_relation_candidate() -> Result<()> {
        let conn = Connection::open_in_memory()?;
        crate::memory::tests_helper::setup_memory_schema(&conn);
        let relation_memory = crate::memory::insert_memory_full(
            &conn,
            None,
            "/repo",
            None,
            "Handles escalation",
            "Handles release escalation for another service.",
            "decision",
            None,
            None,
            "project",
            None,
        )?;
        let subject_memory = crate::memory::insert_memory_full(
            &conn,
            None,
            "/repo",
            None,
            "Rust ownership",
            "Rust is handled by Team Ferris.",
            "decision",
            None,
            None,
            "project",
            None,
        )?;
        let other_subject_memory = crate::memory::insert_memory_full(
            &conn,
            None,
            "/repo",
            None,
            "Rust tooling",
            "Rust uses Cargo for builds.",
            "decision",
            None,
            None,
            "project",
            None,
        )?;
        crate::retrieval::entity::link_entities(&conn, relation_memory, &["Handles".to_string()])?;
        crate::retrieval::entity::link_entities(&conn, subject_memory, &["Rust".to_string()])?;
        crate::retrieval::entity::link_entities(
            &conn,
            other_subject_memory,
            &["Rust".to_string()],
        )?;

        let candidates = entity_scope_candidates("Who Handles Rust?", Some("/repo"));
        let (terms, ids) = select_entity_anchors(
            &conn,
            &candidates,
            Some("/repo"),
            None,
            None,
            5,
            false,
            false,
        )?;

        assert_eq!(terms, vec!["Rust"]);
        assert_eq!(ids, vec![subject_memory, other_subject_memory]);
        Ok(())
    }

    #[test]
    fn claim_matching_uses_token_and_phrase_boundaries() {
        assert!(!claim_term_matches("capitalization", "api"));
        assert!(!claim_term_matches("ghosted deployment", "host"));
        assert!(claim_term_matches(
            "NebulaLatch runs on Oracle Cloud",
            "hosted"
        ));
        assert!(text_contains_exact_token(
            "Team Mica owns NebulaLatch",
            "Team Mica"
        ));
    }

    #[test]
    fn claim_matching_allows_exact_cjk_substrings_without_weakening_latin_boundaries() {
        assert_eq!(
            claim_text_coverage("使用SQLCipher实现数据库加密", &["数据库加密".to_string()]),
            1.0
        );
        assert!(!claim_term_matches("capitalization", "api"));
        assert!(!claim_term_matches("remember", "remem"));
    }

    #[test]
    fn claim_matching_handles_inflection_and_hyphenated_compounds() {
        let text = "Vite deployment supersedes webpack. The queue-runner capture-ledger is active.";
        for term in [
            "deploy",
            "superseded",
            "queue",
            "runner",
            "capture",
            "ledger",
        ] {
            assert!(claim_term_matches(text, term), "missing {term}");
        }
        let preference = "The user prefers concise evidence-backed PR handoffs.";
        for term in ["preference", "evidence", "handoff"] {
            assert!(
                claim_term_matches(preference, term),
                "missing {term} in preference text"
            );
        }
    }

    #[test]
    fn ownership_alias_preserves_multi_hop_owner_evidence() {
        assert!(claim_term_matches(
            "NebulaLatch is owned by Team Mica",
            "owning"
        ));
    }

    #[test]
    fn handler_aliases_cover_roles_but_not_unrelated_entity_text() {
        let claims = vec!["handles".to_string()];

        assert_eq!(
            claim_term_coverage(&test_memory("NebulaLatch is owned by Team Mica."), &claims),
            1.0
        );
        assert_eq!(
            claim_term_coverage(&test_memory("Team Mica uses pager mica-17."), &claims),
            1.0
        );
        assert_eq!(
            claim_term_coverage(&test_memory("NebulaLatch uses SQLite WAL mode."), &claims),
            0.0
        );
    }

    #[test]
    fn multi_hop_owner_and_pager_both_cover_title_case_claims() {
        let claims = vec![
            "pager".to_string(),
            "owning".to_string(),
            "team".to_string(),
        ];
        let mut owner = test_memory("NebulaLatch is owned by Team Mica.");
        assert!(claim_term_coverage(&owner, &claims) >= 0.5);

        owner.text = "Team Mica uses pager mica-17.".to_string();
        assert!(claim_term_coverage(&owner, &claims) >= 0.5);
    }

    fn test_memory(text: &str) -> Memory {
        Memory {
            id: 1,
            session_id: None,
            project: "/repo".to_string(),
            topic_key: None,
            title: String::new(),
            text: text.to_string(),
            memory_type: "decision".to_string(),
            files: None,
            created_at_epoch: 1,
            updated_at_epoch: 1,
            status: "active".to_string(),
            branch: None,
            scope: "project".to_string(),
        }
    }
}
