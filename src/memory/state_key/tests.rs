use super::*;

#[test]
fn stable_topic_key_is_preserved_as_state_key() {
    let decision = derive_state_key(
        "decision",
        Some("deploy-target"),
        "Deploy target",
        "Deploy to staging.",
    )
    .expect("stable topic key should derive");
    assert_eq!(decision.state_key, "deploy-target");
    assert_eq!(decision.reason, "stable_topic_key");
}

#[test]
fn hash_like_topic_key_uses_ascii_preference_domain() {
    let decision = derive_state_key(
        "preference",
        Some("preference-1234abcd"),
        "Preference",
        "Keep verification status separate from data and code changes.",
    )
    .expect("semantic preference should derive");
    assert_eq!(decision.state_key, "verification-status-separation");
}

#[test]
fn hash_like_preference_small_reversible_verified_changes_uses_stable_domain() {
    let Some(first) = derive_state_key(
        "preference",
        Some("preference-11111111"),
        "Preference",
        "Prefers small, reversible changes (“一处改动一个提交”) with concrete verification output: tests, lint, job IDs.",
    ) else {
        panic!("first workflow preference should derive");
    };
    let Some(second) = derive_state_key(
        "preference",
        Some("preference-22222222"),
        "Preference",
        "Prefers one change per commit with concrete evidence from tests, build artifacts, and checklist proof.",
    ) else {
        panic!("second workflow preference should derive");
    };

    assert_eq!(first.state_key, "small-reversible-verified-changes");
    assert_eq!(first.state_key, second.state_key);
    assert_eq!(
        first.reason,
        "preference_domain_small_reversible_verified_changes"
    );
}

#[test]
fn workflow_preference_takes_priority_over_broader_separation_domains() {
    let decision = derive_state_key(
        "preference",
        Some("preference-11111111"),
        "Preference",
        "一处改动，一个提交; include tests and lint output, and keep verification status separate from data/code changes.",
    )
    .expect("workflow preference should derive");

    assert_eq!(decision.state_key, "small-reversible-verified-changes");
}

#[test]
fn workflow_preference_accepts_verified_and_evidence_only_wording() {
    let verified = derive_state_key(
        "preference",
        Some("preference-11111111"),
        "Preference",
        "Use one change per commit, verified with test output and lint logs.",
    )
    .expect("verified workflow preference should derive");
    let evidence_only = derive_state_key(
        "preference",
        Some("preference-22222222"),
        "Preference",
        "Use one change per commit with tests, build artifacts, job IDs, and checklist proof.",
    )
    .expect("evidence-only workflow preference should derive");

    assert_eq!(verified.state_key, "small-reversible-verified-changes");
    assert_eq!(verified.state_key, evidence_only.state_key);
}

#[test]
fn workflow_evidence_terms_are_token_matched() {
    let decision = derive_state_key(
        "preference",
        Some("preference-11111111"),
        "Preference",
        "一处改动一个提交 with verification against the latest docs.",
    );

    assert_ne!(
        decision.map(|decision| decision.state_key),
        Some("small-reversible-verified-changes".to_string())
    );
}

#[test]
fn workflow_with_cumulative_subrules_does_not_share_replacement_key() {
    let first = derive_state_key(
        "preference",
        Some("preference-11111111"),
        "Preference",
        "Prefers small, reversible changes (“一处改动一个提交”) with concrete verification output (tests, lint, job IDs); avoids unsafe content fallbacks.",
    );
    let second = derive_state_key(
        "preference",
        Some("preference-22222222"),
        "Preference",
        "Prefers small, reversible changes (“一处改动一个提交”) with concrete verification output (tests, build, job IDs, artifacts); checklist-driven done only with artifact proof.",
    );

    assert_ne!(
        first.map(|decision| decision.state_key),
        Some("small-reversible-verified-changes".to_string())
    );
    assert_ne!(
        second.map(|decision| decision.state_key),
        Some("small-reversible-verified-changes".to_string())
    );
}

#[test]
fn hash_like_decision_uses_semantic_slot_terms() {
    let Some(decision) = derive_state_key(
        "decision",
        Some("decision-deadbeef"),
        "Optimize CJK search",
        "Use FTS5 trigram tokenizer for CJK text search support.",
    ) else {
        panic!("semantic decision should derive");
    };

    assert_eq!(
        decision.state_key,
        "decision-cjk-fts5-search-tokenizer-trigram"
    );
    assert_eq!(decision.reason, "semantic_slot_terms");
}

#[test]
fn hash_like_decision_paraphrase_uses_same_semantic_slot() {
    let Some(first) = derive_state_key(
        "decision",
        Some("decision-11111111"),
        "Optimize CJK search",
        "Use FTS5 trigram tokenizer for CJK text search support.",
    ) else {
        panic!("first decision should derive");
    };
    let Some(second) = derive_state_key(
        "decision",
        Some("decision-22222222"),
        "Refine CJK search",
        "Switch CJK search to FTS5 trigram tokenization.",
    ) else {
        panic!("second decision should derive");
    };

    assert_eq!(first.state_key, second.state_key);
}

#[test]
fn hash_like_cjk_decision_paraphrase_uses_same_semantic_slot() {
    let Some(first) = derive_state_key(
        "decision",
        Some("decision-11111111"),
        "优化中文搜索",
        "使用三元组分词器支持中文搜索。",
    ) else {
        panic!("first CJK decision should derive");
    };
    let Some(second) = derive_state_key(
        "decision",
        Some("decision-22222222"),
        "调整中文搜索",
        "中文搜索改用三元组分词器。",
    ) else {
        panic!("second CJK decision should derive");
    };

    assert_eq!(first.state_key, "decision-cjk-search-tokenizer-trigram");
    assert_eq!(first.state_key, second.state_key);
}

#[test]
fn truncated_semantic_slot_key_includes_full_term_signature() {
    let vector = derive_state_key(
        "decision",
        Some("decision-11111111"),
        "Vector rotation",
        "Use API auth cache migration token rotation vector.",
    )
    .expect("vector decision should derive");
    let workflow = derive_state_key(
        "decision",
        Some("decision-22222222"),
        "Workflow rotation",
        "Use API auth cache migration token rotation workflow.",
    )
    .expect("workflow decision should derive");

    assert!(vector
        .state_key
        .starts_with("decision-api-auth-cache-migration-rotation-token-"));
    assert!(workflow
        .state_key
        .starts_with("decision-api-auth-cache-migration-rotation-token-"));
    assert_ne!(vector.state_key, workflow.state_key);
}

#[test]
fn cjk_semantic_slot_terms_use_longest_non_overlapping_matches() {
    let decision = derive_state_key(
        "decision",
        Some("decision-11111111"),
        "服务器部署",
        "服务器部署配置端口超时性能验证。",
    )
    .expect("CJK decision should derive");

    assert!(decision
        .state_key
        .starts_with("decision-config-deploy-performance-port-server-timeout-"));
    assert!(
        !decision.state_key.contains("-service-"),
        "服务器 should not also contribute the shorter 服务 term: {}",
        decision.state_key
    );
}

#[test]
fn hash_like_topic_key_uses_cjk_preference_domain() {
    let decision = derive_state_key(
        "preference",
        Some("preference-deadbeef"),
        "Preference",
        "验证状态必须和数据、代码变更分开说明。",
    )
    .expect("CJK semantic preference should derive");
    assert_eq!(decision.state_key, "verification-status-separation");
}

#[test]
fn ambiguous_hash_like_non_preference_is_not_invented() {
    assert!(derive_state_key(
        "decision",
        Some("decision-deadbeef"),
        "Decision",
        "A short ambiguous note.",
    )
    .is_none());
}

#[test]
fn current_memory_id_excludes_expired_active_memory() -> Result<()> {
    let conn = rusqlite::Connection::open_in_memory()?;
    crate::memory::tests_helper::setup_memory_schema(&conn);
    let project = "test/proj";
    let state_key = "repo-test-proj-dev-server";
    let memory_id = crate::memory::insert_memory(
        &conn,
        Some("s1"),
        project,
        Some(state_key),
        "Dev server",
        "Local dev server is currently running at localhost:3000.",
        "decision",
        None,
    )?;
    conn.execute(
        "UPDATE memories SET expires_at_epoch = ?1 WHERE id = ?2",
        rusqlite::params![99_i64, memory_id],
    )?;

    let current = current_memory_id(&conn, "repo", project, "decision", state_key, 100)?;

    assert_eq!(current, None);
    Ok(())
}
