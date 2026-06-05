use anyhow::{anyhow, Result};
use rusqlite::Connection;

use super::{extract_temporal, search_by_time_filtered, TemporalConstraint};
use crate::migrate::MIGRATIONS;
use crate::retrieval::temporal::types::TemporalField;

fn setup_conn() -> Connection {
    let conn = Connection::open_in_memory().expect("in-memory db should open");
    conn.execute_batch(MIGRATIONS[0].sql)
        .expect("baseline schema should load");
    conn.execute_batch("ALTER TABLE memories ADD COLUMN expires_at_epoch INTEGER;")
        .expect("expires column should load");
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
    assert_eq!(
        extract_temporal("recently changed").map(|constraint| constraint.field),
        Some(TemporalField::UpdatedAt)
    );
    assert_eq!(
        extract_temporal("recently SQLite").map(|constraint| constraint.field),
        Some(TemporalField::EventTime)
    );
}

#[test]
fn parse_exact_dates() -> Result<()> {
    let expected_start = chrono::NaiveDate::from_ymd_opt(2026, 5, 4)
        .ok_or_else(|| anyhow!("valid date should construct"))?
        .and_hms_opt(0, 0, 0)
        .ok_or_else(|| anyhow!("valid time should construct"))?
        .and_utc()
        .timestamp();

    for query in [
        "notes from 2026-05-04",
        "notes from May 4, 2026",
        "notes from 4 May 2026",
        "notes from 2026 May 4",
        "notes from 2026年5月4日",
    ] {
        let constraint =
            extract_temporal(query).ok_or_else(|| anyhow!("exact date should parse: {query}"))?;
        assert_eq!(constraint.start_epoch, expected_start);
        assert_eq!(constraint.end_epoch, expected_start + 86_400 - 1);
    }
    Ok(())
}

#[test]
fn parse_month_year_dates() -> Result<()> {
    let expected_start = chrono::NaiveDate::from_ymd_opt(2026, 5, 1)
        .ok_or_else(|| anyhow!("valid date should construct"))?
        .and_hms_opt(0, 0, 0)
        .ok_or_else(|| anyhow!("valid time should construct"))?
        .and_utc()
        .timestamp();
    let expected_end = chrono::NaiveDate::from_ymd_opt(2026, 6, 1)
        .ok_or_else(|| anyhow!("valid date should construct"))?
        .and_hms_opt(0, 0, 0)
        .ok_or_else(|| anyhow!("valid time should construct"))?
        .and_utc()
        .timestamp()
        - 1;

    for query in ["notes from May 2026", "notes from 2026 May"] {
        let constraint =
            extract_temporal(query).ok_or_else(|| anyhow!("month/year should parse: {query}"))?;
        assert_eq!(constraint.start_epoch, expected_start);
        assert_eq!(constraint.end_epoch, expected_end);
    }
    Ok(())
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
            field: TemporalField::EventTime,
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

#[test]
fn search_by_time_uses_created_at_for_event_time() -> Result<()> {
    let conn = setup_conn();
    let now = chrono::Utc::now().timestamp();
    let event_time = now - 30 * 86_400;

    conn.execute(
        "INSERT INTO memories
         (id, session_id, project, topic_key, title, content, memory_type, files, created_at_epoch,
          updated_at_epoch, status, branch, scope)
         VALUES
         (1, NULL, 'alpha', NULL, 'old event', 'backdated event', 'decision', NULL, ?1, ?2, 'active', 'main', 'project')",
        rusqlite::params![event_time, now],
    )?;

    let event_ids = search_by_time_filtered(
        &conn,
        &TemporalConstraint {
            start_epoch: event_time - 10,
            end_epoch: event_time + 10,
            field: TemporalField::EventTime,
        },
        Some("alpha"),
        Some("decision"),
        Some("main"),
        10,
        false,
    )?;
    assert_eq!(event_ids, vec![1]);

    let today_event_ids = search_by_time_filtered(
        &conn,
        &TemporalConstraint {
            start_epoch: now - 10,
            end_epoch: now + 10,
            field: TemporalField::EventTime,
        },
        Some("alpha"),
        Some("decision"),
        Some("main"),
        10,
        false,
    )?;
    assert!(today_event_ids.is_empty());

    let updated_ids = search_by_time_filtered(
        &conn,
        &TemporalConstraint {
            start_epoch: now - 10,
            end_epoch: now + 10,
            field: TemporalField::UpdatedAt,
        },
        Some("alpha"),
        Some("decision"),
        Some("main"),
        10,
        false,
    )?;
    assert_eq!(updated_ids, vec![1]);
    Ok(())
}

#[test]
fn search_by_time_prefers_temporal_fact_event_time() -> Result<()> {
    let conn = setup_conn();
    conn.execute_batch(MIGRATIONS[12].sql)?;
    let now = chrono::Utc::now().timestamp();
    let fact_time = now - 45 * 86_400;

    conn.execute(
        "INSERT INTO memories
         (id, session_id, project, topic_key, title, content, memory_type, files, created_at_epoch,
          updated_at_epoch, status, branch, scope)
         VALUES
         (1, NULL, 'alpha', NULL, 'imported today', 'historical fact', 'decision', NULL, ?1, ?1, 'active', 'main', 'project')",
        rusqlite::params![now],
    )?;
    conn.execute(
        "INSERT INTO memory_facts
         (project, subject, predicate, object, valid_from_epoch, valid_to_epoch,
          learned_at_epoch, source_memory_id, source_observation_id, source_event_ids,
          confidence, supersedes_fact_id, status, created_at_epoch, updated_at_epoch)
         VALUES
         ('alpha', 'historical fact', 'affects_project', 'alpha', ?1, NULL,
          ?2, 1, NULL, '[]', 0.9, NULL, 'active', ?2, ?2)",
        rusqlite::params![fact_time, now],
    )?;

    let fact_ids = search_by_time_filtered(
        &conn,
        &TemporalConstraint {
            start_epoch: fact_time - 10,
            end_epoch: fact_time + 10,
            field: TemporalField::EventTime,
        },
        Some("alpha"),
        Some("decision"),
        Some("main"),
        10,
        false,
    )?;
    assert_eq!(fact_ids, vec![1]);

    let today_ids = search_by_time_filtered(
        &conn,
        &TemporalConstraint {
            start_epoch: now - 10,
            end_epoch: now + 10,
            field: TemporalField::EventTime,
        },
        Some("alpha"),
        Some("decision"),
        Some("main"),
        10,
        false,
    )?;
    assert_eq!(today_ids, vec![1]);
    Ok(())
}

#[test]
fn search_by_time_treats_fact_intervals_as_open_ended_and_exclusive() -> Result<()> {
    let conn = setup_conn();
    conn.execute_batch(MIGRATIONS[12].sql)?;
    let now = chrono::Utc::now().timestamp();
    let may_start = chrono::NaiveDate::from_ymd_opt(2026, 5, 1)
        .ok_or_else(|| anyhow!("valid date should construct"))?
        .and_hms_opt(0, 0, 0)
        .ok_or_else(|| anyhow!("valid time should construct"))?
        .and_utc()
        .timestamp();
    let june_start = chrono::NaiveDate::from_ymd_opt(2026, 6, 1)
        .ok_or_else(|| anyhow!("valid date should construct"))?
        .and_hms_opt(0, 0, 0)
        .ok_or_else(|| anyhow!("valid time should construct"))?
        .and_utc()
        .timestamp();
    let july_start = chrono::NaiveDate::from_ymd_opt(2026, 7, 1)
        .ok_or_else(|| anyhow!("valid date should construct"))?
        .and_hms_opt(0, 0, 0)
        .ok_or_else(|| anyhow!("valid time should construct"))?
        .and_utc()
        .timestamp();

    conn.execute(
        "INSERT INTO memories
         (id, session_id, project, topic_key, title, content, memory_type, files, created_at_epoch,
          updated_at_epoch, status, branch, scope)
         VALUES
         (1, NULL, 'alpha', NULL, 'open ended fact', 'active since May', 'decision', NULL, ?1, ?1, 'active', 'main', 'project'),
         (2, NULL, 'alpha', NULL, 'ended fact', 'ended before June', 'decision', NULL, ?1, ?1, 'active', 'main', 'project')",
        rusqlite::params![now],
    )?;
    conn.execute(
        "INSERT INTO memory_facts
         (project, subject, predicate, object, valid_from_epoch, valid_to_epoch,
          learned_at_epoch, source_memory_id, source_observation_id, source_event_ids,
          confidence, supersedes_fact_id, status, created_at_epoch, updated_at_epoch)
         VALUES
         ('alpha', 'open ended fact', 'affects_project', 'alpha', ?1, NULL,
          ?3, 1, NULL, '[]', 0.9, NULL, 'active', ?3, ?3),
         ('alpha', 'ended fact', 'affects_project', 'alpha', ?1, ?2,
          ?3, 2, NULL, '[]', 0.9, NULL, 'active', ?3, ?3)",
        rusqlite::params![may_start, june_start, now],
    )?;

    let june_ids = search_by_time_filtered(
        &conn,
        &TemporalConstraint {
            start_epoch: june_start,
            end_epoch: july_start - 1,
            field: TemporalField::EventTime,
        },
        Some("alpha"),
        Some("decision"),
        Some("main"),
        10,
        false,
    )?;

    assert!(june_ids.contains(&1), "{june_ids:?}");
    assert!(!june_ids.contains(&2), "{june_ids:?}");
    Ok(())
}

#[test]
fn search_by_time_ignores_stale_temporal_facts() -> Result<()> {
    let conn = setup_conn();
    conn.execute_batch(MIGRATIONS[12].sql)?;
    let now = chrono::Utc::now().timestamp();
    let stale_fact_time = now - 45 * 86_400;

    conn.execute(
        "INSERT INTO memories
         (id, session_id, project, topic_key, title, content, memory_type, files, created_at_epoch,
          updated_at_epoch, status, branch, scope)
         VALUES
         (1, NULL, 'alpha', NULL, 'current event', 'stale fact should not override created_at', 'decision', NULL, ?1, ?1, 'active', 'main', 'project')",
        rusqlite::params![now],
    )?;
    conn.execute(
        "INSERT INTO memory_facts
         (project, subject, predicate, object, valid_from_epoch, valid_to_epoch,
          learned_at_epoch, source_memory_id, source_observation_id, source_event_ids,
          confidence, supersedes_fact_id, status, created_at_epoch, updated_at_epoch)
         VALUES
         ('alpha', 'old stale fact', 'affects_project', 'alpha', ?1, NULL,
          ?2, 1, NULL, '[]', 0.9, NULL, 'stale', ?2, ?2)",
        rusqlite::params![stale_fact_time, now],
    )?;

    let stale_fact_ids = search_by_time_filtered(
        &conn,
        &TemporalConstraint {
            start_epoch: stale_fact_time - 10,
            end_epoch: stale_fact_time + 10,
            field: TemporalField::EventTime,
        },
        Some("alpha"),
        Some("decision"),
        Some("main"),
        10,
        false,
    )?;
    assert!(stale_fact_ids.is_empty());

    let created_at_ids = search_by_time_filtered(
        &conn,
        &TemporalConstraint {
            start_epoch: now - 10,
            end_epoch: now + 10,
            field: TemporalField::EventTime,
        },
        Some("alpha"),
        Some("decision"),
        Some("main"),
        10,
        false,
    )?;
    assert_eq!(created_at_ids, vec![1]);
    Ok(())
}
