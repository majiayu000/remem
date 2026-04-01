use super::super::format::{build_content, build_title, split_into_items, truncate_at_boundary};
use super::super::slug::content_hash;

#[test]
fn test_split_into_items_bullets() {
    let text = "• Use RwLock for concurrent reads\n• Switch to trigram tokenizer\n• Set compression threshold=100";
    let items = split_into_items(text);
    assert_eq!(items.len(), 3);
    assert!(items[0].contains("RwLock"));
}

#[test]
fn test_split_into_items_dashes() {
    let text = "- First decision about architecture\n- Second decision about testing\n- Third one";
    let items = split_into_items(text);
    assert_eq!(items.len(), 3);
}

#[test]
fn test_split_into_items_single_line() {
    let text = "Switched from unicode61 to trigram tokenizer for better CJK support";
    let items = split_into_items(text);
    assert_eq!(items.len(), 1);
}

#[test]
fn test_split_into_items_semicolons() {
    let text = "Use RwLock for concurrent reads; Switch to trigram tokenizer for CJK; Set compression threshold to 100 observations";
    let items = split_into_items(text);
    assert_eq!(items.len(), 3);
}

#[test]
fn test_build_title_from_content() {
    let title = build_title(
        "Use RwLock instead of Mutex for concurrent read support",
        "Optimize search and concurrency",
        "decision",
    );
    assert!(title.contains("RwLock"));
    assert!(title.contains("— decision"));
}

#[test]
fn test_build_title_fallback_to_request() {
    let title = build_title("short", "Optimize search and concurrency", "decision");
    assert!(title.contains("Optimize"));
}

#[test]
fn test_build_content_no_boilerplate() {
    let content = build_content(
        "Use RwLock instead of Mutex for concurrent read support",
        "Optimize search",
    );
    assert!(!content.contains("**Request**"));
    assert!(!content.contains("**Decisions**"));
    assert!(content.contains("[Context:"));
    assert!(content.contains("RwLock"));
}

#[test]
fn test_truncate_at_boundary() {
    let text = "Use RwLock instead of Mutex for concurrent read support in the database layer";
    let truncated = truncate_at_boundary(text, 40);
    assert!(truncated.len() <= 45);
    assert!(!truncated.ends_with(' '));
}

#[test]
fn test_truncate_cjk() {
    let text = "使用 RwLock 替代 Mutex 实现并发读支持。数据库层需要高并发";
    let truncated = truncate_at_boundary(text, 30);
    assert!(truncated.contains("。") || truncated.len() <= 35);
}

#[test]
fn test_truncate_cjk_exact_boundary_panic_regression() {
    let text = "预扣计费模型：整条 DAG 在 Execute 前一次性 reserveAsyncMedia，避免每个 API 节点独立扣费，LLM 节点成本由平台承担";
    let truncated = truncate_at_boundary(text, 107);
    assert!(!truncated.is_empty());
    let _ = content_hash(text);
}
