use crate::entity::extract_entities;

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
