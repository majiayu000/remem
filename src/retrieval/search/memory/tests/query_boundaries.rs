use super::*;

#[test]
fn conversational_scaffolding_does_not_filter_true_owner_claim() -> Result<()> {
    let conn = setup_explain_conn()?;
    let memory = ExplainMemory {
        id: 1,
        project: "/repo",
        title: "NebulaLatch ownership and change",
        content: "NebulaLatch is owned by Team Mica and changed.",
        scope: "project",
        updated_at_epoch: chrono::Utc::now().timestamp(),
    };
    insert_explain_memory(&conn, &memory)?;
    crate::retrieval::entity::link_entities(&conn, 1, &["NebulaLatch".to_string()])?;
    let whether_query = "Please tell me whether NebulaLatch changed";
    for (query, expected_claims) in [
        ("Please tell me who owns NebulaLatch", vec!["owns"]),
        ("Please, tell me who owns NebulaLatch", vec!["owns"]),
        ("Kindly—tell—me who owns NebulaLatch", vec!["owns"]),
        ("Please tell me the owner", vec!["owner"]),
        ("Please tell me about NebulaLatch", vec![]),
        (whether_query, vec!["changed"]),
        ("Please tell me if NebulaLatch changed", vec!["changed"]),
        ("Please tell me NebulaLatch owner", vec!["owner"]),
    ] {
        let (memories, explain) =
            search_with_branch_explain(&conn, Some(query), Some("/repo"), None, 5, 0, false, None)?;
        let explain = explain.context("query explain should be present")?;
        assert_eq!(memories.first().map(|memory| memory.id), Some(1), "{query}");
        assert_eq!(explain.claim_terms, expected_claims, "{query}");
    }
    Ok(())
}

#[test]
fn conversational_words_remain_real_claims_outside_command_prefix() {
    for (query, expected) in [
        ("Does the design please users?", "please"),
        ("Does policy treat users kindly?", "kindly"),
        ("What does the Tell Me feature do?", "tell"),
        ("Please Tell Me feature options", "tell"),
    ] {
        let claims = super::super::claim::query_claim_terms(query, Some("/repo"), &[]);
        assert!(claims.iter().any(|term| term == expected), "{claims:?}");
    }
}

#[test]
fn direct_object_entity_words_include_identifier_characters() {
    for entity in ["X509", "API_v2"] {
        let query = format!("Please tell me {entity} owner");
        let claims =
            super::super::claim::query_claim_terms(&query, Some("/repo"), &[entity.to_string()]);
        assert_eq!(claims, ["owner"], "{query}: {claims:?}");
    }
}
