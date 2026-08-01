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
        ("根因由小王维护吗？", "小王维护根因。"),
        ("数据库根因由小王维护吗？", "小王维护根因，涉及数据库。"),
        ("模块由小王维护吗？", "小王负责维护模块。"),
        ("模块是否由小王维护吗？", "小王维护模块。"),
    ] {
        let claims = super::super::claim::query_claim_terms(query, Some("/repo"), &[]);
        assert_eq!(
            super::super::claim::claim_text_coverage(candidate, &claims),
            1.0,
            "{query}: {claims:?}"
        );
    }

    let claims = super::super::claim::query_claim_terms("模块由小王维护吗？", Some("/repo"), &[]);
    assert_eq!(
        super::super::claim::claim_text_coverage("模块由小李维护；小王负责测试。", &claims),
        0.5,
        "contradictory attribution must remain below the confidence gate: {claims:?}"
    );

    let ambiguous = vec!["模块由小王转由小李".to_string(), "维护".to_string()];
    assert_eq!(
        super::super::claim::claim_text_coverage("小李维护模块由小王转交给团队。", &ambiguous,),
        0.5,
        "multiple grammatical markers must fail closed"
    );
    for (query, candidate) in [
        ("模块由自由软件基金会维护", "自由软件基金会维护模块"),
        ("理由由小王维护", "小王维护理由"),
        ("事由由小王维护", "小王维护事由"),
        ("案由由小王维护", "小王维护案由"),
        ("自由由小王维护", "小王维护自由"),
    ] {
        let claims = super::super::claim::query_claim_terms(query, Some("/repo"), &[]);
        assert_eq!(
            super::super::claim::claim_text_coverage(candidate, &claims),
            1.0,
            "{query}: {claims:?}"
        );
    }
    for (query, unrelated_candidate) in [
        ("自由软件维护吗", "软件维护自动化模块"),
        ("不自由软件维护吗", "软件维护不自动化流程"),
        ("半自由软件维护吗", "软件维护半自动系统"),
        ("理由需要维护吗", "需要维护理想模块"),
        ("根由需要维护吗", "需要维护根目录"),
        ("因由小王维护吗", "小王维护因"),
        ("是否由小王维护吗", "小王维护是否"),
        ("模块是否由于故障维护吗", "故障维护模块"),
        ("模块由小王维护吗", "小王不负责维护模块"),
        ("模块由小王维护吗", "小王协助维护模块"),
        ("模块由小王维护吗", "小王拒绝维护模块"),
        ("模块由小王维护吗", "小王曾负责维护模块"),
        ("模块由小王维护吗", "小王负责测试；维护模块"),
    ] {
        let claims = super::super::claim::query_claim_terms(query, Some("/repo"), &[]);
        assert!(
            super::super::claim::claim_text_coverage(unrelated_candidate, &claims) < 0.62,
            "lexical 由 must not enable relation reordering: {query}: {claims:?}"
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
