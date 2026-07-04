use super::*;

const ENV_KEYS: &[&str] = &[
    "REMEM_CONFIG",
    "REMEM_EMBEDDINGS_PROVIDER",
    "REMEM_EMBEDDING_PROVIDER",
    "REMEM_EMBEDDINGS_MODEL",
    "REMEM_EMBEDDING_MODEL",
    "REMEM_EMBEDDINGS_DIMENSIONS",
    "REMEM_EMBEDDING_DIMENSIONS",
    "REMEM_EMBEDDINGS_FALLBACK",
    "REMEM_EMBEDDINGS_BASE_URL",
    "REMEM_EMBEDDING_BASE_URL",
    "REMEM_EMBEDDINGS_API_KEY",
    "REMEM_EMBEDDING_API_KEY",
    "REMEM_EMBEDDINGS_API_KEY_ENV",
    "REMEM_EMBEDDINGS_TIMEOUT_SECS",
    "REMEM_EMBEDDINGS_MODEL_DIR",
    "OPENAI_API_KEY",
];

struct ScopedEmbeddingProvider {
    _guard: std::sync::MutexGuard<'static, ()>,
    saved: Vec<(&'static str, Option<String>)>,
}

impl ScopedEmbeddingProvider {
    fn new(provider: &str) -> Self {
        let guard = crate::runtime_config::TEST_ENV_LOCK
            .lock()
            .expect("env lock should acquire");
        let saved = ENV_KEYS
            .iter()
            .map(|key| (*key, std::env::var(key).ok()))
            .collect::<Vec<_>>();
        for key in ENV_KEYS {
            unsafe { std::env::remove_var(key) };
        }
        let config_path = std::env::temp_dir().join(format!(
            "remem-preference-consolidation-test-{}-{}.toml",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_nanos())
                .unwrap_or_default()
        ));
        unsafe { std::env::set_var("REMEM_CONFIG", config_path) };
        unsafe { std::env::set_var("REMEM_EMBEDDINGS_PROVIDER", provider) };
        Self {
            _guard: guard,
            saved,
        }
    }
}

impl Drop for ScopedEmbeddingProvider {
    fn drop(&mut self) {
        for (key, value) in self.saved.drain(..) {
            match value {
                Some(value) => unsafe { std::env::set_var(key, value) },
                None => unsafe { std::env::remove_var(key) },
            }
        }
    }
}

fn feature_hash_text_embedding(text: &str) -> TextEmbedding {
    TextEmbedding::new(
        crate::retrieval::embedding::FEATURE_HASH_EMBEDDING_MODEL,
        crate::retrieval::vector::embed_query_text(text),
    )
    .expect("feature-hash test embedding should be valid")
}

fn feature_hash_embedding_refinement(
    memory_id: i64,
    existing: &PreferenceProfile,
    incoming: &PreferenceProfile,
    existing_text: &str,
    incoming_text: &str,
) -> Option<PreferenceConsolidationMatch> {
    let existing_embedding = feature_hash_text_embedding(existing_text);
    let incoming_embedding = feature_hash_text_embedding(incoming_text);
    embedding_refinement_from_embeddings(
        memory_id,
        existing,
        incoming,
        &existing_embedding,
        &incoming_embedding,
    )
}

#[test]
fn consolidation_returns_none_without_embedding_when_no_candidates() -> anyhow::Result<()> {
    let _provider = ScopedEmbeddingProvider::new("api");
    let conn = rusqlite::Connection::open_in_memory()?;
    crate::migrate::run_migrations(&conn)?;

    let result = find_preference_consolidation(
        &conn,
        "repo",
        "/repo",
        "project",
        None,
        "Prefer concise Chinese progress updates.",
        chrono::Utc::now().timestamp(),
    )?;

    assert!(result.is_none());
    Ok(())
}

#[test]
fn consolidation_uses_concepts_before_unavailable_active_embedding() -> anyhow::Result<()> {
    let conn = rusqlite::Connection::open_in_memory()?;
    crate::migrate::run_migrations(&conn)?;
    crate::memory::insert_memory_full(
        &conn,
        None,
        "/repo",
        None,
        "Preference: concise Chinese updates",
        "Prefer concise Chinese progress updates.",
        "preference",
        None,
        None,
        "project",
        None,
    )?;
    let _provider = ScopedEmbeddingProvider::new("api");

    let result = find_preference_consolidation(
        &conn,
        "repo",
        "/repo",
        "project",
        None,
        "Prefer brief Chinese status notes.",
        chrono::Utc::now().timestamp(),
    )?
    .expect("concept match should not require active embedding");

    assert_eq!(result.kind, PreferenceConsolidationKind::Refinement);
    Ok(())
}

#[test]
fn classify_preference_texts_uses_local_fallback_without_active_embedding() {
    let _provider = ScopedEmbeddingProvider::new("api");
    let existing_text = r#"- Prefer minimal vertical slice (最小纵向闭环) over "full cloud platform" first; strict scope control and phased delivery.
- Favor extending existing pathways rather than creating parallel UI/event infrastructure."#;
    let incoming_text = r#"Prefer minimal vertical slice (最小纵向闭环) with deterministic routing, keep live Atlas runs opt-in, and validate via concrete artifacts while keeping credentials server-side."#;

    let result = classify_preference_texts(1, existing_text, incoming_text)
        .expect("render/audit text fallback should stay local");

    assert_eq!(result.kind, PreferenceConsolidationKind::Refinement);
}

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

#[test]
fn embedding_thresholds_are_model_specific() {
    assert_eq!(
        model_embedding_refine_threshold(crate::retrieval::embedding::FEATURE_HASH_EMBEDDING_MODEL),
        FEATURE_HASH_EMBEDDING_REFINE_THRESHOLD
    );
    assert!(
        model_embedding_refine_threshold("fastembed-intfloat-multilingual-e5-small-v1")
            > FEATURE_HASH_EMBEDDING_REFINE_THRESHOLD
    );
    assert!(
        model_embedding_refine_threshold("text-embedding-3-small")
            > FEATURE_HASH_EMBEDDING_REFINE_THRESHOLD
    );
    assert!(
        model_embedding_refine_threshold("vendor/custom-model") > OPENAI_EMBEDDING_REFINE_THRESHOLD
    );
}

#[test]
fn embedding_fallback_refines_same_intent_when_concepts_miss() {
    let existing_text = r#"- Prefer minimal vertical slice (最小纵向闭环) over "full cloud platform" first; strict scope control and phased delivery.
- Favor extending existing pathways rather than creating parallel UI/event infrastructure."#;
    let incoming_text = r#"Prefer minimal vertical slice (最小纵向闭环) with deterministic routing, keep live Atlas runs opt-in, and validate via concrete artifacts while keeping credentials server-side."#;
    let existing = PreferenceProfile::new(existing_text);
    let incoming = PreferenceProfile::new(incoming_text);

    assert!(classify_preference(1, &existing, &incoming).is_none());
    let result = match feature_hash_embedding_refinement(
        1,
        &existing,
        &incoming,
        existing_text,
        incoming_text,
    ) {
        Some(result) => result,
        None => panic!("embedding fallback should match same-intent variants"),
    };

    assert_eq!(result.kind, PreferenceConsolidationKind::Refinement);
    assert!(
        result.score >= FEATURE_HASH_EMBEDDING_REFINE_THRESHOLD as f64,
        "fallback score {} should meet default threshold",
        result.score
    );
}

#[test]
fn embedding_fallback_leaves_unrelated_preferences_unmatched() {
    let existing_text = "Prefer concise Chinese progress updates.";
    let incoming_text =
        "Prefer parameterized SQL queries and reject string-built database statements.";
    let existing = PreferenceProfile::new(existing_text);
    let incoming = PreferenceProfile::new(incoming_text);

    assert!(classify_preference(1, &existing, &incoming).is_none());
    assert!(feature_hash_embedding_refinement(
        1,
        &existing,
        &incoming,
        existing_text,
        incoming_text
    )
    .is_none());
}

#[test]
fn embedding_fallback_rejects_bidirectional_polarity_conflict() {
    let existing_text =
        "Never force push branches; require explicit approval before rewriting history.";
    let incoming_text =
        "Always force push branches; do not require approval before rewriting history.";
    let existing = PreferenceProfile::new(existing_text);
    let incoming = PreferenceProfile::new(incoming_text);

    assert!(feature_hash_embedding_refinement(
        1,
        &existing,
        &incoming,
        existing_text,
        incoming_text
    )
    .is_none());
}

/// Calibration: does main's concept-based consolidation already catch the
/// 10 real "minimal vertical slice" preference variants from the `her`
/// project (2026-05-29)? Run with --nocapture.
#[test]
fn calibrate_her_variants_consolidation_coverage() {
    let variants = [
        r#"- Prefer minimal vertical slice (最小纵向闭环) over "full cloud platform" first; strict scope control and phased delivery (Phase 1 then Phase 2).
    - Favor extending existing pathways (existing `/api/events` + sidebar) rather than creating parallel UI/event infrastructure."#,
        r#"- Prefer minimal vertical slice (最小纵向闭环) and phased delivery; avoid rewriting `/chat` and avoid adding parallel UI/event infrastructure.
- Favor using plugin extension points to avoid bloating core files; validate changes with scoped lint/tests (`npx eslint <file>`, targeted `pytest`)."#,
        r#"- Prefer minimal vertical slice (最小纵向闭环) and phased delivery; avoid rewriting `/chat` and avoid adding parallel UI/event infrastructure.
    - Favor using plugin extension points to avoid bloating core files; validate changes with scoped lint/tests (`npx eslint <file>`, targeted `pytest`).
    - Prefer cost-safe development: mock external providers by default; keep real provider smoke tests explicit opt-in."#,
        r#"- Prefer minimal vertical slice (最小纵向闭环) and phased delivery; avoid rewriting `/chat` or adding parallel UI/event infra. Prefer plugin extension points over core bloat; validate with scoped tests/lints; cost-safe development via mocking external providers by default."#,
        r#"- Prefer minimal vertical slice (最小纵向闭环) and phased delivery; cost-safe development via mocking external providers by default; keep the installed skill surface minimal and deterministic (now single-entry) with tarball backups before deletions."#,
        r#"Prefer minimal vertical slice (最小纵向闭环) and phased delivery; cost-safe development via mocking external providers by default; keep the installed skill surface minimal and deterministic (single entry) and avoid unapproved quota spend (live provider calls only with explicit opt-in)."#,
        r#"Prefer minimal vertical slice (最小纵向闭环) and cost-safe development: mock providers by default, run live Atlas only with explicit opt-in (env `ATLAS_API_KEY`), and keep entrypoints deterministic (single-entry intent routing)."#,
        r#"Prefer minimal vertical slice (最小纵向闭环) with deterministic single-entry routing, keep live Atlas runs opt-in (`ATLAS_API_KEY`), and validate via concrete end-to-end artifacts (HTTP 200 `video/mp4`, Playwright screenshot, test suite pass) rather than dashboard UI integration."#,
        r#"Prefer minimal vertical slice (最小纵向闭环) with deterministic routing, keep `ATLAS_API_KEY` server-side only, and validate via concrete artifacts (tests pass, screenshot, server health) while keeping live Atlas runs opt-in / user-triggered to control cost."#,
        r#"Prefer, cost-safe vertical slices: no auto-start generation, no fake jobs; keep credentials server-side; validate with concrete commands + targeted pytest + real browser verification."#,
    ];
    let profiles: Vec<PreferenceProfile> =
        variants.iter().map(|t| PreferenceProfile::new(t)).collect();
    let mut same = 0;
    let mut refinement = 0;
    let mut contradiction = 0;
    let mut none = 0;
    let mut total = 0;
    for i in 0..profiles.len() {
        for j in (i + 1)..profiles.len() {
            total += 1;
            let result = classify_preference(0, &profiles[i], &profiles[j]).or_else(|| {
                feature_hash_embedding_refinement(
                    0,
                    &profiles[i],
                    &profiles[j],
                    variants[i],
                    variants[j],
                )
            });
            match result {
                Some(m) => match m.kind {
                    PreferenceConsolidationKind::SamePreference => same += 1,
                    PreferenceConsolidationKind::Refinement => refinement += 1,
                    PreferenceConsolidationKind::Contradiction => contradiction += 1,
                },
                None => none += 1,
            }
        }
    }
    println!(
        "her variants ({} pairs): same={}, refinement={}, contradiction={}, none={}",
        total, same, refinement, contradiction, none
    );
    // With embedding fallback, most her variant pairs should now consolidate
    // (concept-only was 40/45 none). none stays non-zero only for the most
    // divergent pair (e.g. 78999, no shared "最小纵向闭环" wording).
    let consolidated = same + refinement + contradiction;
    assert!(
        consolidated >= 40,
        "embedding fallback should consolidate most her variants, got {consolidated}/{total} (none={none})"
    );
}
