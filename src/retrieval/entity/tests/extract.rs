use crate::retrieval::entity::extract_entities;

#[test]
fn extract_tool_names() {
    let entities = extract_entities(
        "FTS5 trigram tokenizer for SQLCipher",
        "Using Rust and Axum",
    );
    assert!(entities.iter().any(|entity| entity.contains("FTS5")));
    assert!(entities
        .iter()
        .any(|entity| entity.to_lowercase() == "sqlcipher"));
    assert!(entities
        .iter()
        .any(|entity| entity.to_lowercase() == "axum"));
}

#[test]
fn extract_from_chinese_mixed() {
    let entities = extract_entities("remem 竞品分析", "对比 Mem0 和 Letta 的设计");
    assert!(entities
        .iter()
        .any(|entity| entity.to_lowercase() == "remem"));
    assert!(entities
        .iter()
        .any(|entity| entity.to_lowercase() == "mem0"));
    assert!(entities
        .iter()
        .any(|entity| entity.to_lowercase() == "letta"));
}

#[test]
fn no_stop_words() {
    let entities = extract_entities("The new API for this", "");
    assert!(!entities.iter().any(|entity| entity.to_lowercase() == "the"));
    assert!(entities.iter().any(|entity| entity == "API"));
}

#[test]
fn technical_terms_do_not_match_inside_words() {
    let entities = extract_entities("", "How can I remember capitalization rules?");

    assert!(!entities
        .iter()
        .any(|entity| entity.eq_ignore_ascii_case("remem")));
    assert!(!entities
        .iter()
        .any(|entity| entity.eq_ignore_ascii_case("API")));
}

#[test]
fn technical_terms_match_at_punctuation_and_hyphen_boundaries() {
    let entities = extract_entities("", "remem-ai uses an API-driven hook.");

    assert!(entities
        .iter()
        .any(|entity| entity.eq_ignore_ascii_case("remem")));
    assert!(entities
        .iter()
        .any(|entity| entity.eq_ignore_ascii_case("API")));
    assert!(entities
        .iter()
        .any(|entity| entity.eq_ignore_ascii_case("hook")));
}

#[test]
fn technical_terms_match_at_cjk_script_boundaries() {
    let entities = extract_entities("", "使用SQLite数据库");

    assert!(entities
        .iter()
        .any(|entity| entity.eq_ignore_ascii_case("sqlite")));
}
