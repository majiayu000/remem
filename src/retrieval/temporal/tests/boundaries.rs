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
        ("在2026-05-04发生什么", " 发生什么"),
        ("2026-05-04发生什么", " 发生什么"),
        ("服务42在2026-05-04发生什么", "服务42 发生什么"),
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
        "服务42:7天前发生了什么",
        "服务42：7天前发生了什么",
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
fn opening_parentheses_separate_entity_numbers_from_day_counts() {
    for (query, expected_semantic_query) in [
        ("服务42（7天前）发生了什么", "服务42（ ）发生了什么"),
        ("service42(7 days ago) changed", "service42( ) changed"),
    ] {
        let constraint = extract_temporal(query)
            .unwrap_or_else(|| panic!("parenthesized day count should parse: {query}"));
        let now = chrono::Utc::now().timestamp();
        assert!((now - constraint.start_epoch - 7 * 86_400).abs() < 2);
        assert_eq!(
            TemporalConstraint::query_without_temporal_expression(query),
            expected_semantic_query,
            "{query}"
        );
    }
}

#[test]
fn opening_parentheses_do_not_override_identifier_joiners() {
    for query in [
        "service42/(7 days ago)",
        "service42\\(7 days ago)",
        "service42_(7 days ago)",
        "service42／（7天前）",
        "service42＼（7天前）",
    ] {
        assert!(extract_temporal(query).is_none(), "{query}");
        assert_eq!(
            TemporalConstraint::query_without_temporal_expression(query),
            query
        );
    }

    let mut failures = Vec::new();
    for joiner in ["_", "＿", "-", "/", "\\", "－", "／", "＼"] {
        for separator_run in [
            format!("{joiner}("),
            format!("{joiner}（"),
            format!(" {joiner}("),
            format!(",{joiner}("),
            format!("（{joiner}("),
        ] {
            for phrase in ["last 7 days", "最近7天"] {
                let query = format!("service42{separator_run}{phrase})");
                if extract_temporal(&query).is_some() {
                    failures.push(query);
                }
            }
        }
    }
    assert!(
        failures.is_empty(),
        "identifier joiners in parenthesis separator runs must fail closed:\n{}",
        failures.join("\n")
    );
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
        ("今天3:30完成", " 完成"),
        ("今天3：30完成", " 完成"),
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
        "今天3:3部署",
        "今天3:60部署",
        "今天24:00部署",
    ] {
        assert!(extract_temporal(query).is_none(), "{query}");
    }
}

#[test]
fn cjk_temporal_phrases_consume_validated_introducers() {
    for (query, expected_semantic_query) in [
        ("服务42自7天前发生了什么", "服务42 发生了什么"),
        ("服务42自从7天前发生了什么", "服务42 发生了什么"),
        ("服务42早在7天前发生了什么", "服务42 发生了什么"),
        ("服务42直到7天前发生了什么", "服务42 发生了什么"),
        ("截至7天前发生了什么", " 发生了什么"),
        ("截止7天前发生了什么", " 发生了什么"),
        ("截至2026年5月4日发生了什么", " 发生了什么"),
        ("截止今天发生了什么", " 发生了什么"),
        ("服务42自从2026年5月4日发生了什么", "服务42 发生了什么"),
        ("截至上周发生了什么", " 发生了什么"),
        ("截止上个月发生了什么", " 发生了什么"),
        ("服务42自从本周发生了什么", "服务42 发生了什么"),
        ("截至最近7天发生了什么", " 发生了什么"),
        ("截止最近发生了什么", " 发生了什么"),
        ("截至 2026年5月4日发生了什么", " 发生了什么"),
        ("截止 今天发生了什么", " 发生了什么"),
        ("服务42自从 上周发生了什么", "服务42 发生了什么"),
        ("截至 最近7天发生了什么", " 发生了什么"),
        ("截止 最近发生了什么", " 发生了什么"),
        ("截至 7天前发生了什么", " 发生了什么"),
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
fn cjk_temporal_introducers_consume_ascii_temporal_phrases() {
    let introducers = [
        "在", "于", "自", "从", "至", "到", "截至", "截止", "自从", "早在", "直到",
    ];
    let separators = ["", " ", "\t", "\n"];
    let temporal_phrases = [
        "May 4, 2026",
        "2026-05-04",
        "today",
        "yesterday",
        "last week",
        "last month",
        "this week",
        "recently",
        "7 days ago",
        "last 7 days",
        "2026年5月4日",
        "今天",
        "昨天",
        "上周",
        "上个月",
        "本周",
        "最近7天",
    ];

    let mut failures = Vec::new();
    for introducer in introducers {
        for separator in separators {
            for phrase in temporal_phrases {
                let query = format!("{introducer}{separator}{phrase} 有什么变化");
                if extract_temporal(&query).is_none() {
                    failures.push(format!("did not parse: {query:?}"));
                    continue;
                }
                let semantic_query = TemporalConstraint::query_without_temporal_expression(&query);
                if semantic_query.contains(introducer) {
                    failures.push(format!(
                        "did not consume introducer: {query:?}: {semantic_query:?}"
                    ));
                }
            }
        }
    }
    assert!(
        failures.is_empty(),
        "temporal introducer matrix failures:\n{}",
        failures.join("\n")
    );
}

#[test]
fn arbitrary_cjk_prefixes_do_not_open_ascii_temporal_boundaries() {
    for query in [
        "服务May 4, 2026 有什么变化",
        "服务7 days ago 有什么变化",
        "服务last 7 days 有什么变化",
    ] {
        assert!(extract_temporal(query).is_none(), "{query}");
        assert_eq!(
            TemporalConstraint::query_without_temporal_expression(query),
            query
        );
    }
}

#[test]
fn ordinary_cjk_suffixes_are_not_consumed_as_temporal_introducers() {
    for query in [
        "存在today notes",
        "关于yesterday notes",
        "来自last week notes",
        "达到recently notes",
    ] {
        assert!(extract_temporal(query).is_none(), "{query}");
        assert_eq!(
            TemporalConstraint::query_without_temporal_expression(query),
            query
        );
    }

    for (query, preserved_prefix) in [
        ("记录存在 May 4, 2026 changed", "记录存在"),
        ("数据来自 last week changed", "数据来自"),
        ("关于 yesterday changed", "关于"),
        ("达到 recently changed", "达到"),
    ] {
        assert!(extract_temporal(query).is_some(), "{query}");
        let semantic_query = TemporalConstraint::query_without_temporal_expression(query);
        assert!(
            semantic_query.contains(preserved_prefix),
            "ordinary CJK prefix must be preserved: {query}: {semantic_query:?}"
        );
    }
}

#[test]
fn temporal_introducers_reject_identifier_joiners_on_their_left() {
    for joiner in ["_", "＿", "-", "/", "\\", "－", "／", "＼"] {
        for phrase in [
            "May 4, 2026",
            "today",
            "yesterday",
            "last week",
            "recently",
            "7 days ago",
            "last 7 days",
            "今天",
            "2026-05-04",
            "最近7天",
        ] {
            for suffix in ["", "_notes"] {
                let query = format!("config{joiner}截至{phrase}{suffix}");
                assert!(extract_temporal(&query).is_none(), "{query}");
                assert_eq!(
                    TemporalConstraint::query_without_temporal_expression(&query),
                    query
                );
            }
        }
    }
}

#[test]
fn fullwidth_underscore_is_an_identifier_boundary() {
    for query in [
        "config＿今天＿notes",
        "config＿2026-05-04＿notes",
        "config＿7天前＿notes",
        "config＿最近7天＿notes",
    ] {
        assert!(extract_temporal(query).is_none(), "{query}");
        assert_eq!(
            TemporalConstraint::query_without_temporal_expression(query),
            query
        );
    }

    for query in ["service＿42,7天前发生了什么", "服务＿42，7天前发生了什么"] {
        assert!(extract_temporal(query).is_some(), "{query}");
    }
}

#[test]
fn fullwidth_identifier_separators_preserve_following_day_counts() {
    for (query, expected_semantic_query) in [
        ("service／42,7天前发生了什么", "service／42, 发生了什么"),
        ("service＼42，7天前发生了什么", "service＼42， 发生了什么"),
        ("service－42,7天前发生了什么", "service－42, 发生了什么"),
        ("服务／42,7天前发生了什么", "服务／42, 发生了什么"),
    ] {
        let constraint = extract_temporal(query)
            .unwrap_or_else(|| panic!("fullwidth identifier day count should parse: {query}"));
        let now = chrono::Utc::now().timestamp();
        assert!((now - constraint.start_epoch - 7 * 86_400).abs() < 2);
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
        ("2026年5月4日3:30发生了什么", " 发生了什么"),
        ("2026年5月4日3：30发生了什么", " 发生了什么"),
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
        "2026年5月4日3:3部署",
        "2026年5月4日3:60部署",
        "2026年5月4日24:00部署",
    ] {
        assert!(extract_temporal(query).is_none(), "{query}");
    }
}

#[test]
fn edge_path_segments_are_not_temporal_phrases() {
    for query in [
        "/today",
        ".today",
        ":today",
        "today/",
        "today\\",
        "today-",
        "config:今天",
        "config：今天",
    ] {
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
