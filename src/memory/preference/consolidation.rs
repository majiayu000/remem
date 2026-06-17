use std::collections::BTreeSet;

use anyhow::{Context, Result};
use rusqlite::{params, Connection};

const ACTIVE_SCAN_LIMIT: i64 = 200;
const MIN_SHARED_CONCEPTS: usize = 3;
const MIN_EXCLUSIVE_SHARED_CONCEPTS: usize = 2;
const MIN_CONTAINMENT: f64 = 0.70;
const MIN_JACCARD: f64 = 0.45;
const MIN_EXCLUSIVE_CONTAINMENT: f64 = 0.60;
const MIN_EXCLUSIVE_JACCARD: f64 = 0.40;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PreferenceConsolidationKind {
    SamePreference,
    Refinement,
    Contradiction,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PreferenceConsolidationMatch {
    pub(crate) memory_id: i64,
    pub(crate) kind: PreferenceConsolidationKind,
    pub(crate) score: f64,
    pub(crate) shared_concepts: Vec<String>,
    pub(crate) reason: String,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn find_preference_consolidation(
    conn: &Connection,
    owner_scope: &str,
    owner_key: &str,
    scope: &str,
    branch: Option<&str>,
    content: &str,
    now_epoch: i64,
) -> Result<Option<PreferenceConsolidationMatch>> {
    let incoming = PreferenceProfile::new(content);
    if incoming.concepts.len() < MIN_SHARED_CONCEPTS {
        return Ok(None);
    }

    let scope = if scope.trim().is_empty() {
        "project"
    } else {
        scope
    };
    let branch = branch.unwrap_or_default();
    let current_filter = crate::memory::memory_state_key_current_filter_sql("m");
    let sql = format!(
        "SELECT m.id, m.content
         FROM memories m
         WHERE m.memory_type = 'preference'
           AND m.status = 'active'
           AND (m.expires_at_epoch IS NULL OR m.expires_at_epoch > ?1)
           AND {current_filter}
           AND COALESCE(m.scope, 'project') = ?2
           AND COALESCE(
                m.owner_scope,
                CASE WHEN COALESCE(m.scope, 'project') = 'global' THEN 'user' ELSE 'repo' END
           ) = ?3
           AND COALESCE(
                m.owner_key,
                CASE WHEN COALESCE(m.scope, 'project') = 'global' THEN 'user:default' ELSE m.project END
           ) = ?4
           AND COALESCE(m.branch, '') = ?5
         ORDER BY m.updated_at_epoch DESC, m.id DESC
         LIMIT ?6"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(
        params![
            now_epoch,
            scope,
            owner_scope,
            owner_key,
            branch,
            ACTIVE_SCAN_LIMIT
        ],
        |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
    )?;
    let candidates = crate::db::query::collect_rows(rows)?;

    let mut best = None;
    for (memory_id, existing_content) in candidates {
        let existing = PreferenceProfile::new(&existing_content);
        let Some(classified) = classify_preference(memory_id, &existing, &incoming) else {
            continue;
        };
        match &best {
            Some(current) if better_match(current, &classified) => {}
            _ => best = Some(classified),
        }
    }
    Ok(best)
}

fn classify_preference(
    memory_id: i64,
    existing: &PreferenceProfile,
    incoming: &PreferenceProfile,
) -> Option<PreferenceConsolidationMatch> {
    if existing.concepts.len() < MIN_SHARED_CONCEPTS {
        return None;
    }

    let shared = existing
        .concepts
        .intersection(&incoming.concepts)
        .cloned()
        .collect::<Vec<_>>();
    let smaller = existing.concepts.len().min(incoming.concepts.len());
    let larger = existing.concepts.len().max(incoming.concepts.len());
    if smaller == 0 || larger == 0 {
        return None;
    }
    let containment = shared.len() as f64 / smaller as f64;
    let union = existing.concepts.union(&incoming.concepts).count();
    let jaccard = shared.len() as f64 / union as f64;
    let score = (containment * 0.7) + (jaccard * 0.3);
    if existing.normalized_text == incoming.normalized_text {
        return Some(PreferenceConsolidationMatch {
            memory_id,
            kind: PreferenceConsolidationKind::SamePreference,
            score,
            shared_concepts: shared.clone(),
            reason: consolidation_reason(
                PreferenceConsolidationKind::SamePreference,
                score,
                &shared,
            ),
        });
    }

    let passes_generic_cutoff = shared.len() >= MIN_SHARED_CONCEPTS
        && containment >= MIN_CONTAINMENT
        && jaccard >= MIN_JACCARD;
    let exclusive_conflict = exclusive_mismatch(existing, incoming);
    let negation_conflict = negation_mismatch(existing, incoming);
    let passes_exclusive_cutoff = shared.len() >= MIN_EXCLUSIVE_SHARED_CONCEPTS
        && containment >= MIN_EXCLUSIVE_CONTAINMENT
        && jaccard >= MIN_EXCLUSIVE_JACCARD;

    if (exclusive_conflict || negation_conflict)
        && (passes_generic_cutoff || passes_exclusive_cutoff)
    {
        return Some(PreferenceConsolidationMatch {
            memory_id,
            kind: PreferenceConsolidationKind::Contradiction,
            score,
            shared_concepts: shared.clone(),
            reason: consolidation_reason(
                PreferenceConsolidationKind::Contradiction,
                score,
                &shared,
            ),
        });
    }

    if !passes_generic_cutoff {
        return None;
    }

    Some(PreferenceConsolidationMatch {
        memory_id,
        kind: PreferenceConsolidationKind::Refinement,
        score,
        shared_concepts: shared.clone(),
        reason: consolidation_reason(PreferenceConsolidationKind::Refinement, score, &shared),
    })
}

fn better_match(
    current: &PreferenceConsolidationMatch,
    candidate: &PreferenceConsolidationMatch,
) -> bool {
    let current_rank = kind_rank(current.kind);
    let candidate_rank = kind_rank(candidate.kind);
    current_rank > candidate_rank
        || (current_rank == candidate_rank && current.score >= candidate.score)
}

fn kind_rank(kind: PreferenceConsolidationKind) -> u8 {
    match kind {
        PreferenceConsolidationKind::SamePreference => 3,
        PreferenceConsolidationKind::Contradiction => 2,
        PreferenceConsolidationKind::Refinement => 1,
    }
}

fn consolidation_reason(
    kind: PreferenceConsolidationKind,
    score: f64,
    shared: &[String],
) -> String {
    format!(
        "generic preference consolidation kind={} score={score:.3} shared=[{}]",
        kind_label(kind),
        shared.join(",")
    )
}

fn kind_label(kind: PreferenceConsolidationKind) -> &'static str {
    match kind {
        PreferenceConsolidationKind::SamePreference => "same_preference",
        PreferenceConsolidationKind::Refinement => "refinement",
        PreferenceConsolidationKind::Contradiction => "contradiction",
    }
}

#[derive(Debug, Clone)]
struct PreferenceProfile {
    normalized_text: String,
    concepts: BTreeSet<String>,
    positive_concepts: BTreeSet<String>,
    negated_concepts: BTreeSet<String>,
}

impl PreferenceProfile {
    fn new(text: &str) -> Self {
        let normalized_text = normalize_preference_text(text);
        let (concepts, positive_concepts, negated_concepts) = preference_concept_profile(text);
        Self {
            normalized_text,
            concepts,
            positive_concepts,
            negated_concepts,
        }
    }
}

fn preference_concept_profile(
    text: &str,
) -> (BTreeSet<String>, BTreeSet<String>, BTreeSet<String>) {
    let mut concepts = BTreeSet::new();
    let mut positive_concepts = BTreeSet::new();
    let mut negated_concepts = BTreeSet::new();
    for clause in text.split([';', '.', '!', '?', '\n', '。', '；']) {
        let clause_concepts = preference_concepts(clause);
        if clause_concepts.is_empty() {
            continue;
        }
        concepts.extend(clause_concepts.iter().cloned());
        if is_negated_preference_clause(clause, &clause_concepts) {
            negated_concepts.extend(clause_concepts);
        } else {
            positive_concepts.extend(clause_concepts);
        }
    }
    if concepts.is_empty() {
        concepts = preference_concepts(text);
        positive_concepts = concepts.clone();
    }
    (concepts, positive_concepts, negated_concepts)
}

fn preference_concepts(text: &str) -> BTreeSet<String> {
    let mut concepts = BTreeSet::new();
    for raw in text.split(|ch: char| !ch.is_ascii_alphanumeric()) {
        let Some(concept) = canonical_concept(raw) else {
            continue;
        };
        concepts.insert(concept);
    }
    add_cjk_concepts(text, &mut concepts);
    concepts
}

fn canonical_concept(raw: &str) -> Option<String> {
    let mut term = raw.trim().to_ascii_lowercase();
    if term.is_empty() {
        return None;
    }
    term = match term.as_str() {
        "updates" | "updated" | "updating" => "update".to_string(),
        "messages" => "message".to_string(),
        "notes" => "note".to_string(),
        "reports" | "reporting" => "report".to_string(),
        "statuses" => "status".to_string(),
        "brief" | "short" | "succinct" => "concise".to_string(),
        "zh" | "cn" => "chinese".to_string(),
        "en" => "english".to_string(),
        "verification" | "verified" | "verifies" | "verify" | "checks" | "checked" => {
            "verification".to_string()
        }
        "tests" | "tested" | "testing" => "test".to_string(),
        "changes" | "changed" | "changing" => "change".to_string(),
        _ => term,
    };
    term = match term.as_str() {
        "progress" | "status" | "update" | "message" | "note" | "report" => "status".to_string(),
        "verification" | "test" | "lint" | "build" | "evidence" | "proof" => {
            "verification".to_string()
        }
        _ => term,
    };
    if is_stopword(&term) {
        return None;
    }
    if term.len() > 4 && term.ends_with('s') && !term.ends_with("ss") && term != "status" {
        term.pop();
    }
    let has_digit = term.chars().any(|ch| ch.is_ascii_digit());
    if term.len() < 3 && !has_digit {
        return None;
    }
    Some(term)
}

fn is_stopword(term: &str) -> bool {
    matches!(
        term,
        "about"
            | "active"
            | "after"
            | "always"
            | "and"
            | "are"
            | "before"
            | "code"
            | "current"
            | "default"
            | "do"
            | "does"
            | "doing"
            | "during"
            | "each"
            | "for"
            | "from"
            | "has"
            | "have"
            | "into"
            | "keep"
            | "long"
            | "must"
            | "not"
            | "only"
            | "please"
            | "prefer"
            | "preferred"
            | "prefers"
            | "project"
            | "provide"
            | "repo"
            | "repository"
            | "should"
            | "task"
            | "the"
            | "this"
            | "through"
            | "to"
            | "use"
            | "user"
            | "when"
            | "while"
            | "with"
            | "without"
            | "work"
            | "workflow"
    )
}

fn add_cjk_concepts(text: &str, concepts: &mut BTreeSet<String>) {
    let mappings = [
        ("中文", "chinese"),
        ("英文", "english"),
        ("进度", "status"),
        ("状态", "status"),
        ("更新", "status"),
        ("消息", "status"),
        ("简洁", "concise"),
        ("简短", "concise"),
        ("验证", "verification"),
        ("测试", "verification"),
        ("证据", "verification"),
    ];
    for (needle, concept) in mappings {
        if text.contains(needle) {
            concepts.insert(concept.to_string());
        }
    }
}

fn is_negated_preference_clause(clause: &str, concepts: &BTreeSet<String>) -> bool {
    if !clause_has_negation(clause) {
        return false;
    }
    concepts.len() >= 2 || has_one_of(concepts, &["chinese", "english"])
}

fn clause_has_negation(text: &str) -> bool {
    let lower_words = format!(" {} ", normalize_preference_text(text));
    lower_words.contains(" do not ")
        || lower_words.contains(" don't ")
        || lower_words.contains(" dont ")
        || lower_words.contains(" never ")
        || lower_words.contains(" avoid ")
        || lower_words.contains(" no ")
        || text.contains("不要")
        || text.contains("避免")
}

fn exclusive_mismatch(existing: &PreferenceProfile, incoming: &PreferenceProfile) -> bool {
    has_one_of(&existing.concepts, &["chinese"]) && has_one_of(&incoming.concepts, &["english"])
        || has_one_of(&existing.concepts, &["english"])
            && has_one_of(&incoming.concepts, &["chinese"])
}

fn negation_mismatch(existing: &PreferenceProfile, incoming: &PreferenceProfile) -> bool {
    !existing
        .negated_concepts
        .is_disjoint(&incoming.positive_concepts)
        || !incoming
            .negated_concepts
            .is_disjoint(&existing.positive_concepts)
}

fn has_one_of(concepts: &BTreeSet<String>, values: &[&str]) -> bool {
    values.iter().any(|value| concepts.contains(*value))
}

fn normalize_preference_text(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

pub(crate) fn load_active_preference_content(conn: &Connection, id: i64) -> Result<String> {
    conn.query_row(
        "SELECT content
         FROM memories
         WHERE id = ?1
           AND memory_type = 'preference'
           AND status = 'active'",
        [id],
        |row| row.get(0),
    )
    .with_context(|| format!("load active preference memory id={id}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_status_update_paraphrase_as_refinement() {
        let existing = PreferenceProfile::new("Prefer concise Chinese progress updates.");
        let incoming = PreferenceProfile::new("Prefer brief Chinese status notes.");

        let result = match classify_preference(1, &existing, &incoming) {
            Some(result) => result,
            None => panic!("should match"),
        };

        assert_eq!(result.kind, PreferenceConsolidationKind::Refinement);
        assert_eq!(
            result.shared_concepts,
            vec![
                "chinese".to_string(),
                "concise".to_string(),
                "status".to_string()
            ]
        );
    }

    #[test]
    fn classifies_negated_same_domain_as_contradiction() {
        let existing = PreferenceProfile::new("Prefer concise Chinese progress updates.");
        let incoming = PreferenceProfile::new("Do not provide brief Chinese status notes.");

        let result = match classify_preference(1, &existing, &incoming) {
            Some(result) => result,
            None => panic!("should match"),
        };

        assert_eq!(result.kind, PreferenceConsolidationKind::Contradiction);
    }

    #[test]
    fn classifies_exclusive_language_swap_as_contradiction_before_generic_cutoff() {
        let existing = PreferenceProfile::new("Prefer concise Chinese progress updates.");
        let incoming = PreferenceProfile::new("Prefer concise English progress updates.");

        let result = match classify_preference(1, &existing, &incoming) {
            Some(result) => result,
            None => panic!("should match"),
        };

        assert_eq!(result.kind, PreferenceConsolidationKind::Contradiction);
    }

    #[test]
    fn local_negation_clause_does_not_reverse_positive_preference() {
        let existing =
            PreferenceProfile::new("Do not be verbose; prefer concise Chinese status notes.");
        let incoming = PreferenceProfile::new("Prefer concise Chinese status notes.");

        let result = match classify_preference(1, &existing, &incoming) {
            Some(result) => result,
            None => panic!("should match"),
        };

        assert_eq!(result.kind, PreferenceConsolidationKind::Refinement);
    }

    #[test]
    fn better_match_prefers_same_preference_over_contradiction() {
        let same = PreferenceConsolidationMatch {
            memory_id: 1,
            kind: PreferenceConsolidationKind::SamePreference,
            score: 0.9,
            shared_concepts: Vec::new(),
            reason: String::new(),
        };
        let contradiction = PreferenceConsolidationMatch {
            memory_id: 2,
            kind: PreferenceConsolidationKind::Contradiction,
            score: 1.0,
            shared_concepts: Vec::new(),
            reason: String::new(),
        };

        assert!(better_match(&same, &contradiction));
        assert!(!better_match(&contradiction, &same));
    }

    #[test]
    fn leaves_generic_but_distinct_preferences_unmatched() {
        let existing = PreferenceProfile::new("Prefer concise Chinese progress updates.");
        let incoming = PreferenceProfile::new("Prefer concise verification logs after tests.");

        assert!(classify_preference(1, &existing, &incoming).is_none());
    }
}
