use rusqlite::Connection;

use super::{extract_temporal, search_by_time_filtered, TemporalConstraint};
use crate::migrate::MIGRATIONS;

fn setup_conn() -> Connection {
    let conn = Connection::open_in_memory().expect("in-memory db should open");
    conn.execute_batch(MIGRATIONS[0].sql)
        .expect("baseline schema should load");
    conn
}

#[test]
fn parse_yesterday() {
    assert!(extract_temporal("yesterday's decisions").is_some());
    assert!(extract_temporal("昨天的决策").is_some());
}

#[test]
fn parse_last_week() {
    assert!(extract_temporal("last week we discussed").is_some());
    assert!(extract_temporal("上周讨论的").is_some());
}

#[test]
fn parse_n_days_ago_en() {
    let constraint = extract_temporal("3 days ago").expect("temporal query should parse");
    let now = chrono::Utc::now().timestamp();
    assert!((now - constraint.start_epoch - 3 * 86_400).abs() < 2);
}

#[test]
fn parse_n_days_ago_cn() {
    assert!(extract_temporal("三天前").is_some());
    assert!(extract_temporal("7天前").is_some());
}

#[test]
fn parse_recently() {
    assert!(extract_temporal("最近的修改").is_some());
    assert!(extract_temporal("recently changed").is_some());
}

#[test]
fn no_temporal_in_normal_query() {
    assert!(extract_temporal("FTS5 search optimization").is_none());
    assert!(extract_temporal("数据库加密").is_none());
}

#[test]
fn search_by_time_filtered_respects_filters() {
    let conn = setup_conn();
    let now = chrono::Utc::now().timestamp();
    let start = now - 100;
    let end = now + 100;

    conn.execute(
        "INSERT INTO memories
         (id, session_id, project, topic_key, title, content, memory_type, files, created_at_epoch,
          updated_at_epoch, status, branch, scope)
         VALUES
         (1, NULL, 'alpha', NULL, 't1', 'c1', 'decision', NULL, ?1, ?1, 'active', 'main', 'project'),
         (2, NULL, 'alpha', NULL, 't2', 'c2', 'decision', NULL, ?2, ?2, 'active', NULL, 'project'),
         (3, NULL, 'alpha', NULL, 't3', 'c3', 'decision', NULL, ?3, ?3, 'archived', 'main', 'project'),
         (4, NULL, 'beta', NULL, 't4', 'c4', 'decision', NULL, ?4, ?4, 'active', 'main', 'project')",
        rusqlite::params![now - 10, now - 20, now - 30, now - 40],
    )
    .expect("memories should insert");

    let ids = search_by_time_filtered(
        &conn,
        &TemporalConstraint {
            start_epoch: start,
            end_epoch: end,
        },
        Some("alpha"),
        Some("decision"),
        Some("main"),
        10,
        false,
    )
    .expect("time search should succeed");

    assert_eq!(ids, vec![1, 2]);
}
