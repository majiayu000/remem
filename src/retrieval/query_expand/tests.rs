use super::tokenize::tokenize_mixed;
use super::{core_tokens, expand_query};

#[test]
fn expand_english_to_chinese() {
    let expanded = expand_query("encrypt");
    assert!(expanded.contains(&"加密".to_string()));
    assert!(!expanded.contains(&"encryption".to_string()));
    assert!(!expanded.contains(&"sqlcipher".to_string()));
}

#[test]
fn expand_chinese_to_english() {
    let expanded = expand_query("数据库");
    assert!(expanded.contains(&"database".to_string()));
    assert!(expanded.contains(&"db".to_string()));
    assert!(!expanded.contains(&"sqlite".to_string()));
}

#[test]
fn expand_multi_token() {
    let expanded = expand_query("数据库 加密");
    assert!(expanded.contains(&"database".to_string()));
    assert!(expanded.contains(&"encrypt".to_string()));
}

#[test]
fn no_duplicates() {
    let expanded = expand_query("encrypt encryption");
    let count = expanded
        .iter()
        .filter(|token| token.to_lowercase() == "encrypt")
        .count();
    assert_eq!(count, 1);
}

#[test]
fn unknown_word_passes_through() {
    let expanded = expand_query("foobar");
    assert_eq!(expanded, vec!["foobar"]);
}

#[test]
fn cjk_segmentation_database_encrypt() {
    let expanded = expand_query("数据库加密");
    assert!(
        expanded.contains(&"数据库".to_string()),
        "should segment 数据库: {:?}",
        expanded
    );
    assert!(
        expanded.contains(&"加密".to_string()),
        "should segment 加密: {:?}",
        expanded
    );
    assert!(
        expanded.contains(&"database".to_string()),
        "should expand 数据库→database: {:?}",
        expanded
    );
    assert!(
        expanded.contains(&"encrypt".to_string()),
        "should expand 加密→encrypt: {:?}",
        expanded
    );
}

#[test]
fn cjk_segmentation_cross_project_sharing() {
    let expanded = expand_query("跨项目记忆共享");
    assert!(
        expanded.contains(&"跨项目".to_string()),
        "should segment 跨项目: {:?}",
        expanded
    );
    assert!(
        expanded.contains(&"记忆".to_string()),
        "should segment 记忆: {:?}",
        expanded
    );
    assert!(
        expanded.contains(&"共享".to_string()),
        "should segment 共享: {:?}",
        expanded
    );
}

#[test]
fn cjk_segmentation_memory_quality() {
    let expanded = expand_query("记忆质量");
    assert!(
        expanded.contains(&"记忆".to_string()),
        "should segment 记忆: {:?}",
        expanded
    );
    assert!(
        expanded.contains(&"质量".to_string()),
        "should segment 质量: {:?}",
        expanded
    );
    assert!(
        expanded.contains(&"memory".to_string()),
        "should expand to memory: {:?}",
        expanded
    );
    assert!(
        expanded.contains(&"quality".to_string()),
        "should expand to quality: {:?}",
        expanded
    );
}

#[test]
fn retired_monolingual_expansions_do_not_expand() {
    let expanded = expand_query("search");
    assert!(expanded.contains(&"搜索".to_string()));
    assert!(!expanded.contains(&"fts".to_string()));
    assert!(!expanded.contains(&"查询".to_string()));
}

#[test]
fn mixed_cjk_and_ascii() {
    let expanded = expand_query("Claude Code hook 机制");
    assert!(expanded.contains(&"Claude".to_string()));
    assert!(expanded.contains(&"Code".to_string()));
    assert!(expanded.contains(&"hook".to_string()));
    assert!(expanded.contains(&"机制".to_string()));
}

#[test]
fn tokenize_mixed_test() {
    let tokens = tokenize_mixed("数据库加密test");
    assert_eq!(tokens, vec!["数据库加密", "test"]);
    assert_eq!(tokenize_mixed("last_30_days"), vec!["last_30_days"]);
    assert_eq!(tokenize_mixed("3_days_ago"), vec!["3_days_ago"]);
}

#[test]
fn core_tokens_preserve_unknown_cjk_qualifier_spans() {
    let tokens = core_tokens("谁负责港湾服务欧洲生产环境？");

    assert!(
        tokens.contains(&"欧洲生产环境".to_string()),
        "unknown CJK qualifiers must remain claim evidence: {tokens:?}"
    );
}

#[test]
fn core_tokens_preserve_short_mixed_script_qualifiers() {
    let tokens = core_tokens("谁验证了港湾服务A区？");

    assert!(
        tokens.contains(&"A区".to_string()),
        "short mixed-script qualifiers must remain one claim token: {tokens:?}"
    );
}

#[test]
fn compact_mixed_identifiers_do_not_cross_boundaries_or_leading_cjk() {
    assert!(tokenize_mixed("A区").contains(&"A区".to_string()));
    assert!(!tokenize_mixed("42于").contains(&"42于".to_string()));
    assert!(!tokenize_mixed("A 区").contains(&"A区".to_string()));
    assert!(!tokenize_mixed("A-区").contains(&"A区".to_string()));
    assert!(!tokenize_mixed("在EU").contains(&"在EU".to_string()));
}
