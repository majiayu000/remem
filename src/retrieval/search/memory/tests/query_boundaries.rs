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
        ("Today, please tell me who owns NebulaLatch", vec!["owns"]),
        ("Today: please tell me who owns NebulaLatch", vec!["owns"]),
        ("Today. please tell me who owns NebulaLatch", vec!["owns"]),
        ("Today： please tell me who owns NebulaLatch", vec!["owns"]),
        ("Today． please tell me who owns NebulaLatch", vec!["owns"]),
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
fn empty_conversational_prefixes_do_not_clear_all_claims() {
    for query in [
        "Please tell me about?",
        "Please tell me if!",
        "Please tell me whether.",
        "Please tell me who?",
        "Please tell me the?",
        "Please tell me about the?",
        "Please tell me whether the?",
        "Please tell me who the?",
        "Please tell me the a?",
    ] {
        let claims = super::super::claim::query_claim_terms(query, Some("/repo"), &[]);
        assert!(
            !claims.is_empty(),
            "empty command subject must retain abstention evidence: {query}"
        );
    }
}

#[test]
fn cjk_relational_claims_match_reordered_candidates_without_losing_qualifiers() {
    for (query, candidate) in [
        ("模块由小王维护吗？", "小王维护模块。"),
        ("库由小王维护吗？", "小王维护库。"),
        ("模块由王维护吗？", "王维护模块。"),
    ] {
        let claims = super::super::claim::query_claim_terms(query, Some("/repo"), &[]);
        assert_eq!(
            super::super::claim::claim_text_coverage(candidate, &claims),
            1.0,
            "{query}: {claims:?}"
        );
    }

    let claims = super::super::claim::query_claim_terms("EU模块由小王维护R2", Some("/repo"), &[]);
    assert!(claims.contains(&"eu".to_string()), "{claims:?}");
    assert!(claims.contains(&"r2".to_string()), "{claims:?}");
}

#[test]
fn terminal_cjk_question_particles_do_not_pollute_claims() {
    let claims = super::super::claim::query_claim_terms(
        "NebulaLatch运行正常吗？",
        Some("/repo"),
        &["NebulaLatch".to_string()],
    );
    assert_eq!(claims, ["运行正常"]);
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

#[test]
fn structural_separators_do_not_form_conversational_prefixes() {
    for separator in [".", "/", "\\", "-", ":", "．", "／", "＼", "－", "："] {
        for position in 0..3 {
            let mut gaps = [" ", " ", " "];
            gaps[position] = separator;
            let query = format!(
                "please{}tell{}me{}about NebulaLatch",
                gaps[0], gaps[1], gaps[2]
            );
            let claims = super::super::claim::query_claim_terms(&query, Some("/repo"), &[]);
            assert!(
                claims.iter().any(|term| term.contains("please")),
                "structural query must remain semantic: {query}: {claims:?}"
            );
        }
    }

    let underscore = "please_tell_me_about NebulaLatch";
    let claims = super::super::claim::query_claim_terms(underscore, Some("/repo"), &[]);
    assert!(
        claims.iter().any(|term| term.contains("please")),
        "{claims:?}"
    );
}
