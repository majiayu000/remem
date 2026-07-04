use std::collections::BTreeSet;

use anyhow::{Context, Result};
use rusqlite::{params, Connection};

use crate::retrieval::embedding::TextEmbedding;

const ACTIVE_SCAN_LIMIT: i64 = 200;
const MIN_SHARED_CONCEPTS: usize = 3;
const MIN_EXCLUSIVE_SHARED_CONCEPTS: usize = 2;
const MIN_CONTAINMENT: f64 = 0.70;
const MIN_JACCARD: f64 = 0.45;
const MIN_EXCLUSIVE_CONTAINMENT: f64 = 0.60;
const MIN_EXCLUSIVE_JACCARD: f64 = 0.40;

/// Feature-hash threshold for the embedding fallback when concept-based
/// classification returns None. Calibrated on real `her` "minimal vertical
/// slice" variants (2026-05-29): concept consolidation missed 89% of them
/// (jaccard<0.45 because each variant adds distinct detail words), while
/// feature-hash embedding separates them (min pairwise cosine 0.621 vs
/// unrelated max 0.435). 0.55 sits inside that gap.
const FEATURE_HASH_EMBEDDING_REFINE_THRESHOLD: f32 = 0.55;
const MULTILINGUAL_E5_EMBEDDING_REFINE_THRESHOLD: f32 = 0.78;
const BGE_M3_EMBEDDING_REFINE_THRESHOLD: f32 = 0.80;
const OPENAI_EMBEDDING_REFINE_THRESHOLD: f32 = 0.82;
const UNKNOWN_MODEL_EMBEDDING_REFINE_THRESHOLD: f32 = 0.90;

fn embedding_refine_threshold(model: &str) -> f32 {
    std::env::var("REMEM_PREF_EMBEDDING_THRESHOLD")
        .ok()
        .and_then(|s| s.parse::<f32>().ok())
        .filter(|v| (0.0..=1.0).contains(v))
        .unwrap_or_else(|| model_embedding_refine_threshold(model))
}

fn model_embedding_refine_threshold(model: &str) -> f32 {
    match model {
        crate::retrieval::embedding::FEATURE_HASH_EMBEDDING_MODEL => {
            FEATURE_HASH_EMBEDDING_REFINE_THRESHOLD
        }
        "fastembed-intfloat-multilingual-e5-small-v1" => MULTILINGUAL_E5_EMBEDDING_REFINE_THRESHOLD,
        "fastembed-bge-m3-v1" => BGE_M3_EMBEDDING_REFINE_THRESHOLD,
        model if model.starts_with("text-embedding-3-") => OPENAI_EMBEDDING_REFINE_THRESHOLD,
        _ => UNKNOWN_MODEL_EMBEDDING_REFINE_THRESHOLD,
    }
}

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
    if candidates.is_empty() {
        return Ok(None);
    }

    let mut best = None;
    let candidates = candidates
        .into_iter()
        .map(|(memory_id, existing_content)| {
            let existing = PreferenceProfile::new(&existing_content);
            (memory_id, existing_content, existing)
        })
        .collect::<Vec<_>>();
    for (memory_id, _existing_content, existing) in &candidates {
        let Some(classified) = classify_preference(*memory_id, existing, &incoming) else {
            continue;
        };
        match &best {
            Some(current) if better_match(current, &classified) => {}
            _ => best = Some(classified),
        }
    }
    if matches!(
        best.as_ref().map(|matched| matched.kind),
        Some(PreferenceConsolidationKind::SamePreference)
            | Some(PreferenceConsolidationKind::Contradiction)
    ) {
        return Ok(best);
    }

    let mut fallback_cache = crate::retrieval::embedding::EmbeddingFallbackCache::default();
    let mut incoming_embedding =
        active_preference_embedding_with_fallback_cache(content, &mut fallback_cache)?;
    if incoming_embedding.is_none() {
        return Ok(best);
    }
    for (memory_id, existing_content, existing) in candidates {
        let Some(classified) = embedding_refinement(
            memory_id,
            &existing,
            &incoming,
            &existing_content,
            content,
            &mut incoming_embedding,
            &mut fallback_cache,
        )?
        else {
            continue;
        };
        match &best {
            Some(current) if better_match(current, &classified) => {}
            _ => best = Some(classified),
        }
    }
    Ok(best)
}

pub(crate) fn classify_preference_texts(
    memory_id: i64,
    existing_content: &str,
    incoming_content: &str,
) -> Option<PreferenceConsolidationMatch> {
    let existing = PreferenceProfile::new(existing_content);
    let incoming = PreferenceProfile::new(incoming_content);
    if existing.normalized_text == incoming.normalized_text {
        return Some(PreferenceConsolidationMatch {
            memory_id,
            kind: PreferenceConsolidationKind::SamePreference,
            score: 1.0,
            shared_concepts: existing
                .concepts
                .intersection(&incoming.concepts)
                .cloned()
                .collect(),
            reason: "normalized preference text already matches".to_string(),
        });
    }
    if incoming.concepts.len() < MIN_SHARED_CONCEPTS {
        return None;
    }
    if let Some(classified) = classify_preference(memory_id, &existing, &incoming) {
        return Some(classified);
    }
    let existing_embedding = match feature_hash_preference_embedding(existing_content) {
        Ok(embedding) => embedding,
        Err(_) => return None,
    };
    let incoming_embedding = match feature_hash_preference_embedding(incoming_content) {
        Ok(embedding) => embedding,
        Err(_) => return None,
    };
    embedding_refinement_from_embeddings(
        memory_id,
        &existing,
        &incoming,
        &existing_embedding,
        &incoming_embedding,
    )
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

fn active_preference_embedding_with_fallback_cache(
    text: &str,
    fallback_cache: &mut crate::retrieval::embedding::EmbeddingFallbackCache,
) -> Result<Option<TextEmbedding>> {
    #[cfg(test)]
    if active_preference_embedding_is_forbidden() {
        anyhow::bail!("active preference embedding called in forbidden test scope");
    }
    match crate::retrieval::embedding::embed_query_with_fallback_cache(text, fallback_cache) {
        Ok(embedding) => Ok(Some(embedding)),
        Err(error) if crate::retrieval::embedding::is_embedding_provider_off_error(&error) => {
            if configured_embedding_provider_is_off()? {
                Ok(None)
            } else {
                Err(error).context("active preference embedding provider failed")
            }
        }
        Err(error) => Err(error).context("active preference embedding provider failed"),
    }
}

fn configured_embedding_provider_is_off() -> Result<bool> {
    Ok(
        crate::retrieval::embedding::embedding_provider_status_without_probe()?.configured_provider
            == "off",
    )
}

#[cfg(test)]
thread_local! {
    static FORBID_ACTIVE_PREFERENCE_EMBEDDING: std::cell::Cell<bool> =
        const { std::cell::Cell::new(false) };
}

#[cfg(test)]
fn active_preference_embedding_is_forbidden() -> bool {
    FORBID_ACTIVE_PREFERENCE_EMBEDDING.with(|flag| flag.get())
}

#[cfg(test)]
fn with_forbidden_active_preference_embedding<T>(f: impl FnOnce() -> T) -> T {
    struct ResetForbiddenFlag(bool);

    impl Drop for ResetForbiddenFlag {
        fn drop(&mut self) {
            FORBID_ACTIVE_PREFERENCE_EMBEDDING.with(|flag| flag.set(self.0));
        }
    }

    let previous = FORBID_ACTIVE_PREFERENCE_EMBEDDING.with(|flag| {
        let previous = flag.get();
        flag.set(true);
        previous
    });
    let _reset = ResetForbiddenFlag(previous);
    f()
}

fn feature_hash_preference_embedding(text: &str) -> Result<TextEmbedding> {
    TextEmbedding::new(
        crate::retrieval::embedding::FEATURE_HASH_EMBEDDING_MODEL,
        crate::retrieval::vector::embed_query_text(text),
    )
}

/// Embedding-cosine fallback for refinement when concept-based classification
/// misses (concepts diverge but intent matches). Only reached when
/// classify_preference returns None, so concept Contradiction/SamePreference
/// always win first.
fn embedding_refinement(
    memory_id: i64,
    existing: &PreferenceProfile,
    incoming: &PreferenceProfile,
    existing_content: &str,
    incoming_content: &str,
    incoming_embedding: &mut Option<TextEmbedding>,
    fallback_cache: &mut crate::retrieval::embedding::EmbeddingFallbackCache,
) -> Result<Option<PreferenceConsolidationMatch>> {
    if incoming_embedding.is_none() {
        return Ok(None);
    }
    let Some(existing_embedding) =
        active_preference_embedding_with_fallback_cache(existing_content, fallback_cache)
            .with_context(|| format!("embed active preference candidate id={memory_id}"))?
    else {
        return Ok(None);
    };
    if let Some(current_incoming) = incoming_embedding.as_ref() {
        if existing_embedding.model() != current_incoming.model()
            || existing_embedding.dimensions() != current_incoming.dimensions()
        {
            *incoming_embedding =
                active_preference_embedding_with_fallback_cache(incoming_content, fallback_cache)
                    .context("re-embed incoming preference after provider fallback")?;
        }
    }
    let Some(incoming_embedding) = incoming_embedding.as_ref() else {
        return Ok(None);
    };
    Ok(embedding_refinement_from_embeddings(
        memory_id,
        existing,
        incoming,
        &existing_embedding,
        incoming_embedding,
    ))
}

fn embedding_refinement_from_embeddings(
    memory_id: i64,
    existing: &PreferenceProfile,
    incoming: &PreferenceProfile,
    existing_embedding: &TextEmbedding,
    incoming_embedding: &TextEmbedding,
) -> Option<PreferenceConsolidationMatch> {
    if existing_embedding.model() != incoming_embedding.model()
        || existing_embedding.dimensions() != incoming_embedding.dimensions()
    {
        return None;
    }
    embedding_refinement_from_vectors(
        memory_id,
        existing,
        incoming,
        existing_embedding.model(),
        existing_embedding.values(),
        incoming_embedding.values(),
    )
}

fn embedding_refinement_from_vectors(
    memory_id: i64,
    existing: &PreferenceProfile,
    incoming: &PreferenceProfile,
    model: &str,
    existing_embedding: &[f32],
    incoming_embedding: &[f32],
) -> Option<PreferenceConsolidationMatch> {
    // Embedding cosine can't see negation/polarity, so never merge across a
    // polarity conflict even if wording overlaps highly (e.g. "never force push"
    // vs "always force push"). Require a BIDIRECTIONAL conflict (each side
    // negates something the other asserts): a single-direction overlap is
    // usually the coarse clause-level negation rule mislabeling a positive
    // sub-clause (e.g. "favor plugin extension points to avoid bloating core"
    // marks "plugin" as negated), not a genuine opposite.
    let polarity_conflict = !existing
        .negated_concepts
        .is_disjoint(&incoming.positive_concepts)
        && !incoming
            .negated_concepts
            .is_disjoint(&existing.positive_concepts);
    if exclusive_mismatch(existing, incoming) || polarity_conflict {
        return None;
    }
    let distance =
        crate::retrieval::vector::cosine_distance(incoming_embedding, existing_embedding).ok()?;
    let cosine = 1.0 - distance as f64;
    let threshold = embedding_refine_threshold(model);
    if cosine >= threshold as f64 {
        Some(PreferenceConsolidationMatch {
            memory_id,
            kind: PreferenceConsolidationKind::Refinement,
            score: cosine,
            shared_concepts: Vec::new(),
            reason: format!(
                "embedding cosine={cosine:.3} model={model} threshold={threshold:.3} refinement (concept cutoff missed)"
            ),
        })
    } else {
        None
    }
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
mod tests;
