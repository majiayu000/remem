use super::expand_query;
use super::tokenize::tokenize_mixed;

#[test]
fn expand_english_to_chinese() {
    let expanded = expand_query("encrypt");
    assert!(expanded.contains(&"加密".to_string()));
    assert!(expanded.contains(&"sqlcipher".to_string()));
}

#[test]
fn expand_chinese_to_english() {
    let expanded = expand_query("数据库");
    assert!(expanded.contains(&"database".to_string()));
    assert!(expanded.contains(&"sqlite".to_string()));
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
}
