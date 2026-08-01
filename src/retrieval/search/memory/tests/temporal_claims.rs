use super::*;

#[test]
fn parsed_temporal_number_does_not_filter_temporal_results() -> Result<()> {
    let conn = setup_explain_conn()?;
    let now = chrono::Utc::now().timestamp();
    insert_explain_memory(
        &conn,
        &ExplainMemory {
            id: 1,
            project: "/repo",
            title: "Deployment update",
            content: "Deployment configuration changed.",
            scope: "project",
            updated_at_epoch: now - 60,
        },
    )?;

    let (memories, explain) = search_with_branch_explain(
        &conn,
        Some("What changed in the last 30 days?"),
        Some("/repo"),
        None,
        5,
        0,
        false,
        None,
    )?;
    let explain = explain.context("query explain should be present")?;

    assert_eq!(memories.first().map(|memory| memory.id), Some(1));
    assert!(explain.temporal_range.is_some());
    assert_eq!(explain.claim_terms, vec!["changed"]);
    Ok(())
}

#[test]
fn cjk_temporal_scaffolding_is_absent_from_search_claims() -> Result<()> {
    let now = chrono::Utc::now().timestamp();
    let exact_date = chrono::NaiveDate::from_ymd_opt(2026, 5, 4)
        .context("exact date should construct")?
        .and_hms_opt(12, 0, 0)
        .context("exact time should construct")?
        .and_utc()
        .timestamp();
    for (query, memory_epoch, expected, days) in [
        ("今天有什么变化", now, vec!["变化"], 0),
        ("2026年5月4日有什么变化", exact_date, vec!["变化"], 0),
        ("最近30天有什么变化", now, vec!["变化"], 30),
        (
            "服务42于30天前有什么变化？",
            now - 29 * 86_400 - 43_200,
            vec!["服务", "42", "变化"],
            30,
        ),
        (
            "服务42早在7天前有什么变化？",
            now - 6 * 86_400 - 43_200,
            vec!["服务", "42", "变化"],
            7,
        ),
        (
            "服务42直到7天前有什么变化？",
            now - 6 * 86_400 - 43_200,
            vec!["服务", "42", "变化"],
            7,
        ),
        ("最近的天气", now, vec!["天气"], 3),
        ("最近每天", now, vec!["每天"], 3),
        ("最近的部署每天", now, vec!["部署", "每天"], 3),
    ] {
        let conn = setup_explain_conn()?;
        insert_explain_memory(
            &conn,
            &ExplainMemory {
                id: 1,
                project: "/repo",
                title: "服务42部署变化 天气 每天",
                content: "配置已更新。",
                scope: "project",
                updated_at_epoch: memory_epoch,
            },
        )?;
        let before_epoch = chrono::Utc::now().timestamp();
        let (memories, explain) =
            search_with_branch_explain(&conn, Some(query), Some("/repo"), None, 5, 0, false, None)?;
        let after_epoch = chrono::Utc::now().timestamp();
        let explain = explain.context("query explain should be present")?;
        assert_eq!(memories.first().map(|memory| memory.id), Some(1), "{query}");
        assert_eq!(explain.claim_terms, expected, "{query}");
        if days > 0 {
            let (start, _) = explain
                .temporal_range
                .context("recent query should expose its range")?;
            assert!((before_epoch - days * 86_400..=after_epoch - days * 86_400).contains(&start));
        }
    }
    Ok(())
}

#[test]
fn cjk_temporal_introducers_do_not_compact_with_entity_numbers() {
    for introducer in [
        "在", "于", "自", "从", "至", "到", "截至", "截止", "自从", "早在", "直到",
    ] {
        let query = format!("服务42{introducer}7天前有什么变化？");
        let claims = super::super::claim::query_claim_terms(&query, Some("/repo"), &[]);
        let compact = format!("42{introducer}");
        assert!(
            claims.iter().any(|term| term == "42"),
            "{query}: {claims:?}"
        );
        assert!(
            !claims.iter().any(|term| term == &compact),
            "temporal introducer must not become an entity claim: {query}: {claims:?}"
        );
        assert!(
            !claims.iter().any(|term| term == introducer),
            "temporal introducer must not remain an independent claim: {query}: {claims:?}"
        );
    }

    for query in [
        "截至 2026年5月4日有什么变化？",
        "截止 今天有什么变化？",
        "服务42自从 上周有什么变化？",
        "截至 最近7天有什么变化？",
        "截止 最近有什么变化？",
        "截至 7天前有什么变化？",
        "截至 May 4, 2026 有什么变化？",
        "截止 yesterday 有什么变化？",
        "服务42自从 last week 有什么变化？",
        "截至 last month 有什么变化？",
        "截至 recently 有什么变化？",
        "截至 7 days ago 有什么变化？",
        "截至 last 7 days 有什么变化？",
    ] {
        let claims = super::super::claim::query_claim_terms(query, Some("/repo"), &[]);
        assert!(
            !claims
                .iter()
                .any(|term| ["截至", "截止", "自从"].contains(&term.as_str())),
            "spaced temporal introducer must not remain a claim: {query}: {claims:?}"
        );
    }
}

#[test]
fn ordinary_cjk_words_ending_in_introducer_characters_remain_claims() {
    for (query, expected_claim) in [
        ("记录存在 May 4, 2026 changed", "记录存在"),
        ("数据来自 last week changed", "数据来自"),
        ("关于 yesterday changed", "关于"),
        ("达到 recently changed", "达到"),
    ] {
        let claims = super::super::claim::query_claim_terms(query, Some("/repo"), &[]);
        assert!(
            claims.iter().any(|term| term == expected_claim),
            "ordinary CJK word must remain a claim: {query}: {claims:?}"
        );
    }
}

#[test]
fn non_temporal_number_remains_a_required_claim_in_temporal_query() -> Result<()> {
    let conn = setup_explain_conn()?;
    let now = chrono::Utc::now().timestamp();
    for (id, content) in [(1, "Service 42 changed."), (2, "Incident 17 changed.")] {
        insert_explain_memory(
            &conn,
            &ExplainMemory {
                id,
                project: "/repo",
                title: "Deployment update",
                content,
                scope: "project",
                updated_at_epoch: now - 60,
            },
        )?;
    }

    for (query, expected_id, expected_claims, days) in [
        (
            "What changed for service 42 in the last 30 days?",
            1,
            vec!["changed", "service", "42"],
            30,
        ),
        ("service 42, 30 days ago", 1, vec!["service", "42"], 30),
        ("service 42 30 days ago", 1, vec!["service", "42"], 30),
        ("incident 17—3 days ago", 2, vec!["incident", "17"], 3),
    ] {
        let before_epoch = chrono::Utc::now().timestamp();
        let (memories, explain) =
            search_with_branch_explain(&conn, Some(query), Some("/repo"), None, 5, 0, false, None)?;
        let after_epoch = chrono::Utc::now().timestamp();
        let explain = explain.context("query explain should be present")?;
        let (start, _) = explain
            .temporal_range
            .context("temporal range should parse")?;
        assert_eq!(memories.first().map(|memory| memory.id), Some(expected_id));
        assert_eq!(explain.claim_terms, expected_claims, "{query}");
        assert!((before_epoch - days * 86_400..=after_epoch - days * 86_400).contains(&start));
    }
    Ok(())
}
