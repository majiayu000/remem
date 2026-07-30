use super::*;
use rusqlite::params;

fn fact_scope_fixture() -> Result<Connection> {
    let conn = Connection::open_in_memory()?;
    crate::migrate::run_migrations(&conn)?;
    let now = chrono::Utc::now().timestamp();
    for id in 1..=6 {
        crate::memory::insert_memory_full(
            &conn,
            None,
            "/repo",
            None,
            "fact",
            "source",
            "decision",
            None,
            None,
            "project",
            Some(now - id),
        )?;
    }
    conn.execute(
        "INSERT INTO memory_facts
         (project, subject, predicate, object, learned_at_epoch, source_memory_id,
          source_event_ids, confidence, status, created_at_epoch, updated_at_epoch)
         VALUES ('/repo', 'HarborMint', 'verified_by', 'Toma Reed', ?1, 1, '[]', 0.9,
                 'active', ?1, ?1),
                ('/repo', 'OtherService', 'verified_by', 'Toma Reed', ?1, 2, '[]', 0.9,
                 'active', ?1, ?1),
                ('/repo', 'HarborMint', 'verified_by', 'Old Reed', ?1, 2, '[]', 0.9,
                 'stale', ?1, ?1),
                ('/repo', 'HarborMint', 'verified_by', 'Mira Lane', ?1, 3, '[]', 0.9,
                 'active', ?1, ?1),
                ('/repo', 'HarborMint', 'verified_by', 'Toma Reed', ?1, 4, '[]', 0.2,
                 'active', ?1, ?1),
                ('/repo', '港湾服务', 'verified_by', '林舟', ?1, 5, '[]', 0.9,
                 'active', ?1, ?1),
                ('/repo', 'HarborMint', 'uses_command', 'cargo test', ?1, 6, '[]', 0.9,
                 'active', ?1, ?1)",
        params![now - 10],
    )?;
    Ok(conn)
}

#[test]
fn structured_fact_scope_requires_current_exact_entity_binding() -> Result<()> {
    let conn = fact_scope_fixture()?;
    let scope = structured_fact_scope(
        &conn,
        &[1, 2, 3, 4],
        "Who verified HarborMint with Toma Reed?",
        &["HarborMint".to_string()],
        &[
            "verified".to_string(),
            "toma".to_string(),
            "reed".to_string(),
        ],
        0.6,
        Some("/repo"),
        crate::retrieval::temporal::FactTimeMode::Current,
    )?;

    assert_eq!(scope.bound_ids, HashSet::from([1, 4]));
    assert_eq!(scope.supported_ids, HashSet::from([1]));
    Ok(())
}

#[test]
fn structured_fact_scope_rejects_vacuous_typo_binding() -> Result<()> {
    let conn = fact_scope_fixture()?;
    let scope = structured_fact_scope(
        &conn,
        &[1, 2, 3, 4],
        "Who verified a service with Moma Reed?",
        &[],
        &[
            "verified".to_string(),
            "moma".to_string(),
            "reed".to_string(),
        ],
        0.6,
        Some("/repo"),
        crate::retrieval::temporal::FactTimeMode::Current,
    )?;

    assert!(scope.bound_ids.is_empty());
    assert!(scope.supported_ids.is_empty());
    Ok(())
}

#[test]
fn structured_fact_scope_requires_full_claim_when_entity_inference_is_partial() -> Result<()> {
    let conn = fact_scope_fixture()?;
    let scope = structured_fact_scope(
        &conn,
        &[1, 2, 3, 4],
        "who verified harbormint with moma reed?",
        &[],
        &[
            "verified".to_string(),
            "harbormint".to_string(),
            "moma".to_string(),
            "reed".to_string(),
        ],
        0.6,
        Some("/repo"),
        crate::retrieval::temporal::FactTimeMode::Current,
    )?;

    assert!(scope.supported_ids.is_empty());
    Ok(())
}

#[test]
fn structured_fact_scope_binds_cjk_subject_inside_unspaced_query() -> Result<()> {
    let conn = fact_scope_fixture()?;
    let scope = structured_fact_scope(
        &conn,
        &[5],
        "谁验证了港湾服务？",
        &[],
        &[],
        0.6,
        Some("/repo"),
        crate::retrieval::temporal::FactTimeMode::Current,
    )?;

    assert_eq!(scope.bound_ids, HashSet::from([5]));
    assert_eq!(scope.supported_ids, HashSet::from([5]));
    Ok(())
}

#[test]
fn structured_fact_scope_binds_two_character_cjk_object() -> Result<()> {
    let conn = fact_scope_fixture()?;
    let query = "林舟验证了什么？";
    let core_terms = crate::retrieval::query_expand::core_tokens(query);
    let claim_terms = super::super::super::claim::claim_terms(&core_terms, Some("/repo"), &[]);
    let scope = structured_fact_scope(
        &conn,
        &[5],
        query,
        &[],
        &claim_terms,
        0.6,
        Some("/repo"),
        crate::retrieval::temporal::FactTimeMode::Current,
    )?;

    assert_eq!(scope.bound_ids, HashSet::from([5]));
    assert_eq!(scope.supported_ids, HashSet::from([5]));
    Ok(())
}

#[test]
fn structured_fact_scope_rejects_missing_cjk_qualifier() -> Result<()> {
    let conn = fact_scope_fixture()?;
    let query = "谁验证了港湾服务欧洲生产环境？";
    let core_terms = crate::retrieval::query_expand::core_tokens(query);
    let claim_terms = super::super::super::claim::claim_terms(&core_terms, Some("/repo"), &[]);
    let scope = structured_fact_scope(
        &conn,
        &[5],
        query,
        &[],
        &claim_terms,
        0.6,
        Some("/repo"),
        crate::retrieval::temporal::FactTimeMode::Current,
    )?;

    assert!(
        claim_terms.contains(&"欧洲生产环境".to_string()),
        "qualifier must survive into claim terms: {claim_terms:?}"
    );
    assert_eq!(scope.bound_ids, HashSet::from([5]));
    assert!(scope.supported_ids.is_empty());
    Ok(())
}

#[test]
fn structured_fact_scope_keeps_supported_cjk_relation_without_extra_qualifier() -> Result<()> {
    let conn = fact_scope_fixture()?;
    let query = "谁验证了港湾服务？";
    let core_terms = crate::retrieval::query_expand::core_tokens(query);
    let claim_terms = super::super::super::claim::claim_terms(&core_terms, Some("/repo"), &[]);
    let scope = structured_fact_scope(
        &conn,
        &[5],
        query,
        &[],
        &claim_terms,
        0.6,
        Some("/repo"),
        crate::retrieval::temporal::FactTimeMode::Current,
    )?;

    assert_eq!(scope.bound_ids, HashSet::from([5]));
    assert_eq!(scope.supported_ids, HashSet::from([5]));
    Ok(())
}

#[test]
fn structured_fact_scope_rejects_missing_short_script_qualifiers() -> Result<()> {
    let conn = fact_scope_fixture()?;
    for (query, expected_qualifier) in [
        ("谁验证了港湾服务A区？", "a区"),
        ("谁验证了港湾服务 EU？", "eu"),
    ] {
        let core_terms = crate::retrieval::query_expand::core_tokens(query);
        let claim_terms = super::super::super::claim::claim_terms(&core_terms, Some("/repo"), &[]);
        let scope = structured_fact_scope(
            &conn,
            &[5],
            query,
            &[],
            &claim_terms,
            0.6,
            Some("/repo"),
            crate::retrieval::temporal::FactTimeMode::Current,
        )?;

        assert!(
            claim_terms.iter().any(|term| term == expected_qualifier),
            "qualifier {expected_qualifier} must survive: {claim_terms:?}"
        );
        assert_eq!(scope.bound_ids, HashSet::from([5]));
        assert!(
            scope.supported_ids.is_empty(),
            "broader fact must not support {query}"
        );
    }
    Ok(())
}

#[test]
fn structured_fact_scope_accepts_matching_short_script_qualifiers() -> Result<()> {
    let conn = fact_scope_fixture()?;
    let now = chrono::Utc::now().timestamp();
    for (subject, query, expected_qualifier) in [
        ("港湾服务A区", "谁验证了港湾服务A区？", "a区"),
        ("HarborMint EU", "Who verified HarborMint EU?", "eu"),
    ] {
        let source_memory_id = crate::memory::insert_memory_full(
            &conn,
            None,
            "/repo",
            None,
            subject,
            "qualified source",
            "decision",
            None,
            None,
            "project",
            Some(now),
        )?;
        conn.execute(
            "INSERT INTO memory_facts
             (project, subject, predicate, object, learned_at_epoch, source_memory_id,
              source_event_ids, confidence, status, created_at_epoch, updated_at_epoch)
             VALUES (?1, ?2, 'verified_by', '林舟', ?3, ?4, '[]', 0.9,
                     'active', ?3, ?3)",
            params!["/repo", subject, now, source_memory_id],
        )?;
        let core_terms = crate::retrieval::query_expand::core_tokens(query);
        let claim_terms = super::super::super::claim::claim_terms(&core_terms, Some("/repo"), &[]);
        let scope = structured_fact_scope(
            &conn,
            &[source_memory_id],
            query,
            &[],
            &claim_terms,
            0.6,
            Some("/repo"),
            crate::retrieval::temporal::FactTimeMode::Current,
        )?;

        assert!(
            claim_terms.iter().any(|term| term == expected_qualifier),
            "qualifier {expected_qualifier} must survive: {claim_terms:?}"
        );
        assert_eq!(scope.supported_ids, HashSet::from([source_memory_id]));
    }
    Ok(())
}

#[test]
fn structured_fact_scope_allows_cjk_current_time_modifier() -> Result<()> {
    let conn = fact_scope_fixture()?;
    for modifier in ["当前", "目前", "最近"] {
        let query = format!("{modifier}谁验证了港湾服务？");
        let core_terms = crate::retrieval::query_expand::core_tokens(&query);
        let claim_terms = super::super::super::claim::claim_terms(&core_terms, Some("/repo"), &[]);
        let scope = structured_fact_scope(
            &conn,
            &[5],
            &query,
            &[],
            &claim_terms,
            0.6,
            Some("/repo"),
            crate::retrieval::temporal::FactTimeMode::Current,
        )?;

        assert!(
            !claim_terms.iter().any(|term| term == modifier),
            "{modifier} must remain query-level temporal syntax, not claim evidence: {claim_terms:?}"
        );
        assert_eq!(scope.supported_ids, HashSet::from([5]));
    }
    Ok(())
}

#[test]
fn structured_fact_scope_allows_modifier_gap_after_full_fact_value_binding() -> Result<()> {
    let conn = fact_scope_fixture()?;
    let scope = structured_fact_scope(
        &conn,
        &[1, 2, 3, 4],
        "who recently verified harbormint with toma reed?",
        &[],
        &[
            "recently".to_string(),
            "verified".to_string(),
            "harbormint".to_string(),
            "toma".to_string(),
            "reed".to_string(),
        ],
        0.6,
        Some("/repo"),
        crate::retrieval::temporal::FactTimeMode::Current,
    )?;

    assert_eq!(scope.supported_ids, HashSet::from([1]));
    Ok(())
}

#[test]
fn structured_fact_scope_rejects_typo_even_with_explicit_subject_binding() -> Result<()> {
    let conn = fact_scope_fixture()?;
    let scope = structured_fact_scope(
        &conn,
        &[1, 2, 3, 4],
        "Who verified HarborMint with Moma Reed?",
        &["HarborMint".to_string()],
        &[
            "verified".to_string(),
            "moma".to_string(),
            "reed".to_string(),
        ],
        0.6,
        Some("/repo"),
        crate::retrieval::temporal::FactTimeMode::Current,
    )?;

    assert!(scope.supported_ids.is_empty());
    Ok(())
}

#[test]
fn structured_fact_scope_rejects_extra_unbound_entity_after_complete_binding() -> Result<()> {
    let conn = fact_scope_fixture()?;
    let scope = structured_fact_scope(
        &conn,
        &[1, 2, 3, 4],
        "Who verified HarborMint with Toma Reed and Moma Reed?",
        &["HarborMint".to_string()],
        &[
            "verified".to_string(),
            "toma".to_string(),
            "reed".to_string(),
            "moma".to_string(),
        ],
        0.6,
        Some("/repo"),
        crate::retrieval::temporal::FactTimeMode::Current,
    )?;

    assert!(scope.supported_ids.is_empty());
    Ok(())
}

#[test]
fn structured_fact_scope_rejects_lowercase_extra_unbound_entity() -> Result<()> {
    let conn = fact_scope_fixture()?;
    let scope = structured_fact_scope(
        &conn,
        &[1, 2, 3, 4],
        "who verified harbormint with toma reed and moma reed?",
        &[],
        &[
            "verified".to_string(),
            "harbormint".to_string(),
            "toma".to_string(),
            "reed".to_string(),
            "moma".to_string(),
        ],
        0.6,
        Some("/repo"),
        crate::retrieval::temporal::FactTimeMode::Current,
    )?;

    assert!(scope.supported_ids.is_empty());
    Ok(())
}

#[test]
fn structured_fact_scope_rejects_deleted_query_for_verified_fact() -> Result<()> {
    let conn = fact_scope_fixture()?;
    let scope = structured_fact_scope(
        &conn,
        &[1, 2, 3, 4],
        "Who deleted HarborMint with Toma Reed?",
        &["HarborMint".to_string()],
        &[
            "deleted".to_string(),
            "toma".to_string(),
            "reed".to_string(),
        ],
        0.6,
        Some("/repo"),
        crate::retrieval::temporal::FactTimeMode::Current,
    )?;

    assert!(scope.supported_ids.is_empty());
    Ok(())
}

#[test]
fn structured_fact_scope_rejects_cjk_delete_for_verified_fact() -> Result<()> {
    let conn = fact_scope_fixture()?;
    let scope = structured_fact_scope(
        &conn,
        &[5],
        "谁删除了港湾服务？",
        &[],
        &[],
        0.6,
        Some("/repo"),
        crate::retrieval::temporal::FactTimeMode::Current,
    )?;

    assert!(scope.supported_ids.is_empty());
    Ok(())
}

#[test]
fn structured_fact_scope_accepts_title_case_uses_for_compatible_fact() -> Result<()> {
    let conn = fact_scope_fixture()?;
    let scope = structured_fact_scope(
        &conn,
        &[6],
        "Who Uses HarborMint with cargo test?",
        &["HarborMint".to_string()],
        &["uses".to_string(), "cargo".to_string(), "test".to_string()],
        0.6,
        Some("/repo"),
        crate::retrieval::temporal::FactTimeMode::Current,
    )?;

    assert_eq!(scope.supported_ids, HashSet::from([6]));
    Ok(())
}
