use anyhow::{anyhow, Result};

use super::super::{extract_temporal, TemporalConstraint};

#[test]
fn separated_exact_dates_accept_cjk_sentence_context() -> Result<()> {
    let expected_start = chrono::NaiveDate::from_ymd_opt(2026, 5, 4)
        .ok_or_else(|| anyhow!("valid date should construct"))?
        .and_hms_opt(0, 0, 0)
        .ok_or_else(|| anyhow!("valid time should construct"))?
        .and_utc()
        .timestamp();
    for (query, expected_semantic_query) in [
        ("在2026-05-04发生什么", "在 发生什么"),
        ("2026-05-04发生什么", " 发生什么"),
        ("服务42在2026-05-04发生什么", "服务42在 发生什么"),
    ] {
        let constraint =
            extract_temporal(query).ok_or_else(|| anyhow!("CJK sentence date should parse"))?;
        assert_eq!(constraint.start_epoch, expected_start);
        assert_eq!(
            TemporalConstraint::query_without_temporal_expression(query),
            expected_semantic_query
        );
    }
    Ok(())
}

#[test]
fn month_name_date_identifiers_are_not_temporal_expressions() {
    for query in [
        "release/May/4/2026/notes",
        "release.May.4.2026.notes",
        "release/4/May/2026/notes",
    ] {
        assert!(extract_temporal(query).is_none(), "{query}");
        assert_eq!(
            TemporalConstraint::query_without_temporal_expression(query),
            query
        );
    }
}

#[test]
fn invalid_cjk_relative_identifier_does_not_hide_later_phrase() {
    for query in [
        "config_7天前_notes，服务在3天前变化",
        "config_最近7天_notes，服务最近3天有变化",
    ] {
        let constraint = extract_temporal(query).expect("later CJK phrase should parse");
        let now = chrono::Utc::now().timestamp();
        assert!((now - constraint.start_epoch - 3 * 86_400).abs() < 2);
    }
}

#[test]
fn cjk_clause_punctuation_separates_entity_number_from_day_count() {
    for (query, days, expected_semantic_query) in [
        ("服务42，7天前发生了什么", 7, "服务42， 发生了什么"),
        ("服务42，365天前发生了什么", 365, "服务42， 发生了什么"),
        ("服务42,7天前发生了什么", 7, "服务42, 发生了什么"),
        ("service_42，7天前发生了什么", 7, "service_42， 发生了什么"),
        ("service-42,7天前发生了什么", 7, "service-42, 发生了什么"),
        ("service/42,7天前发生了什么", 7, "service/42, 发生了什么"),
        ("服务42;7天前发生了什么", 7, "服务42; 发生了什么"),
        ("服务42:7天前发生了什么", 7, "服务42: 发生了什么"),
        ("服务42!7天前发生了什么", 7, "服务42! 发生了什么"),
        ("服务42?7天前发生了什么", 7, "服务42? 发生了什么"),
    ] {
        let constraint = extract_temporal(query)
            .unwrap_or_else(|| panic!("CJK clause day count should parse: {query}"));
        let now = chrono::Utc::now().timestamp();
        assert!((now - constraint.start_epoch - days * 86_400).abs() < 2);
        assert_eq!(
            TemporalConstraint::query_without_temporal_expression(query),
            expected_semantic_query
        );
    }

    for grouped in [
        "2，030天前",
        "12，345天前",
        "2，30天前",
        "29，5天前",
        "2,030天前",
        "v7天前",
    ] {
        assert!(extract_temporal(grouped).is_none(), "{grouped}");
        assert_eq!(
            TemporalConstraint::query_without_temporal_expression(grouped),
            grouped
        );
    }
}

#[test]
fn sentence_punctuation_preserves_temporal_phrases() {
    for query in [
        "What changed today.",
        "Today: what changed?",
        "What changed May 4, 2026.",
    ] {
        assert!(extract_temporal(query).is_some(), "{query}");
    }
}

#[test]
fn exact_dates_accept_clause_punctuation() {
    for query in [
        "2026-05-04: what changed?",
        "2026-05-04. What changed?",
        "2026.05.04. What changed?",
        "2026年5月4日：发生了什么",
    ] {
        assert!(extract_temporal(query).is_some(), "{query}");
    }
}

#[test]
fn compact_month_name_dates_accept_bare_year_comma() {
    let query = "What changed on May 4,2026?";
    assert!(extract_temporal(query).is_some(), "{query}");
    let semantic_query = TemporalConstraint::query_without_temporal_expression(query);
    assert!(!semantic_query.contains("May"), "{semantic_query:?}");
    assert!(!semantic_query.contains("2026"), "{semantic_query:?}");
}

#[test]
fn cjk_day_phrases_accept_valid_clock_suffixes() {
    for (query, expected_semantic_query) in [
        ("今天3点发生什么", " 发生什么"),
        ("今天 3点发生什么", " 发生什么"),
        ("昨天2点的部署", " 的部署"),
        ("今天3点半完成", " 完成"),
        ("今天3点钟完成", " 完成"),
        ("今天3点整完成", " 完成"),
        ("今天23时59分完成", " 完成"),
    ] {
        assert!(extract_temporal(query).is_some(), "{query}");
        assert_eq!(
            TemporalConstraint::query_without_temporal_expression(query),
            expected_semantic_query,
            "{query}"
        );
    }

    for query in [
        "今天２",
        "今天 ２点部署",
        "今天三点部署",
        "今天 三点部署",
        "今天3点Status",
        "今天3点_notes",
        "今天24点部署",
    ] {
        assert!(extract_temporal(query).is_none(), "{query}");
    }
}

#[test]
fn cjk_day_counts_consume_validated_temporal_introducers() {
    for (query, expected_semantic_query) in [
        ("服务42自7天前发生了什么", "服务42 发生了什么"),
        ("服务42自从7天前发生了什么", "服务42 发生了什么"),
        ("截至7天前发生了什么", " 发生了什么"),
        ("截止7天前发生了什么", " 发生了什么"),
    ] {
        assert!(extract_temporal(query).is_some(), "{query}");
        assert_eq!(
            TemporalConstraint::query_without_temporal_expression(query),
            expected_semantic_query,
            "{query}"
        );
    }
}

#[test]
fn exact_dates_accept_valid_compact_cjk_clock_suffixes() {
    for (query, expected_semantic_query) in [
        ("2026年5月4日3点发生了什么", " 发生了什么"),
        ("2026年5月4日 3点发生了什么", " 发生了什么"),
        ("2026年5月4日23时59分发生了什么", " 发生了什么"),
        ("2026-05-043点发生了什么", " 发生了什么"),
    ] {
        assert!(extract_temporal(query).is_some(), "{query}");
        assert_eq!(
            TemporalConstraint::query_without_temporal_expression(query),
            expected_semantic_query,
            "{query}"
        );
    }

    for query in [
        "2026年5月4日２点",
        "2026年5月4日24点部署",
        "2026年5月4日 24点部署",
    ] {
        assert!(extract_temporal(query).is_none(), "{query}");
    }
}

#[test]
fn edge_path_segments_are_not_temporal_phrases() {
    for query in ["/today", ".today", ":today", "today/", "today\\", "today-"] {
        assert!(extract_temporal(query).is_none(), "{query}");
        assert_eq!(
            TemporalConstraint::query_without_temporal_expression(query),
            query
        );
    }
}

#[test]
fn generic_recent_scans_past_invalid_quantified_identifier() {
    let query = "config_最近7天_notes，最近有什么变化";
    let constraint = extract_temporal(query).expect("later generic recent phrase should parse");
    let now = chrono::Utc::now().timestamp();
    assert!((now - constraint.start_epoch - 3 * 86_400).abs() < 2);
    assert_eq!(
        TemporalConstraint::query_without_temporal_expression(query),
        "config_最近7天_notes， 有什么变化"
    );
}
