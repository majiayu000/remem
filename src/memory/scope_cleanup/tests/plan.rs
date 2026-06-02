use anyhow::Result;
use rusqlite::{params, Connection};
use sha2::{Digest, Sha256};

use crate::memory::scope_cleanup::{
    apply_memory_cleanup_plan, build_preference_cleanup_plan, MemoryCleanupPlan,
    MemoryCleanupRowSnapshot,
};

use super::{seed_stash_pollution, setup_conn, STASH};

#[test]
fn cleanup_plan_detects_ascii_and_cjk_duplicates_without_mutation() -> Result<()> {
    let conn = setup_conn();
    insert_pref(
        &conn,
        2100,
        STASH,
        "Preference: verify before claim",
        "Always run fresh verification before claiming completion.",
        Some("pref-a"),
        "repo",
        STASH,
    )?;
    insert_pref(
        &conn,
        2101,
        STASH,
        "Preference: fresh verification",
        "Always run fresh verification before claiming completion.",
        Some("pref-b"),
        "repo",
        STASH,
    )?;
    insert_pref(
        &conn,
        2110,
        STASH,
        "Preference: 中文验收",
        "提交前必须运行最新测试并说明结果。",
        Some("pref-c"),
        "repo",
        STASH,
    )?;
    insert_pref(
        &conn,
        2111,
        STASH,
        "Preference: 测试说明",
        "提交前必须运行最新测试并说明结果。",
        Some("pref-d"),
        "repo",
        STASH,
    )?;

    let plan = build_preference_cleanup_plan(&conn, STASH)?;

    assert_eq!(plan.groups.len(), 2);
    assert!(plan
        .groups
        .iter()
        .any(|group| group.current_id == 2101 && group.stale_ids == vec![2100]));
    assert!(plan
        .groups
        .iter()
        .any(|group| group.current_id == 2111 && group.stale_ids == vec![2110]));
    assert_active(&conn, &[2100, 2101, 2110, 2111])?;
    Ok(())
}

#[test]
fn cleanup_apply_stales_plan_ids_and_writes_audit() -> Result<()> {
    let conn = setup_conn();
    seed_stash_pollution(&conn);

    let plan = build_preference_cleanup_plan(&conn, STASH)?;
    let encoded = serde_json::to_string_pretty(&plan)?;
    let decoded: MemoryCleanupPlan = serde_json::from_str(&encoded)?;
    assert_eq!(decoded, plan);

    let result = apply_memory_cleanup_plan(&conn, &decoded)?;

    assert_eq!(result.groups_applied, 1);
    assert_eq!(result.current_ids, vec![1032]);
    assert_eq!(result.stale_ids, vec![1030, 1031]);
    assert_eq!(
        conn.query_row("SELECT status FROM memories WHERE id = 1032", [], |row| {
            row.get::<_, String>(0)
        })?,
        "active"
    );
    for id in [1030, 1031] {
        assert_eq!(
            conn.query_row("SELECT status FROM memories WHERE id = ?1", [id], |row| {
                row.get::<_, String>(0)
            })?,
            "stale"
        );
    }
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM memory_operation_log
             WHERE source = 'memory_cleanup' AND planner_version = 'memory-cleanup-v1'",
            [],
            |row| row.get::<_, i64>(0)
        )?,
        1
    );
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM memory_edges
             WHERE edge_type = 'duplicates' AND source_operation_id IS NOT NULL",
            [],
            |row| row.get::<_, i64>(0)
        )?,
        2
    );
    Ok(())
}

#[test]
fn cleanup_apply_rejects_changed_rows() -> Result<()> {
    let conn = setup_conn();
    seed_stash_pollution(&conn);
    let plan = build_preference_cleanup_plan(&conn, STASH)?;

    conn.execute(
        "UPDATE memories SET content = ?1, updated_at_epoch = updated_at_epoch + 1 WHERE id = 1031",
        ["changed after dry-run"],
    )?;
    let err = apply_memory_cleanup_plan(&conn, &plan).expect_err("changed rows must be rejected");

    assert!(err.to_string().contains("changed since dry-run"));
    assert_active(&conn, &[1030, 1031, 1032])?;
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM memory_operation_log", [], |row| {
            row.get::<_, i64>(0)
        })?,
        0
    );
    Ok(())
}

#[test]
fn cleanup_apply_rejects_plan_project_mismatch() -> Result<()> {
    let conn = setup_conn();
    seed_stash_pollution(&conn);
    let mut plan = build_preference_cleanup_plan(&conn, STASH)?;
    plan.project = "/tmp/other-project".to_string();

    let err = apply_memory_cleanup_plan(&conn, &plan).expect_err("project mismatch must fail");

    assert!(err.to_string().contains("does not belong to project"));
    assert_active(&conn, &[1030, 1031, 1032])?;
    Ok(())
}

#[test]
fn cleanup_apply_rejects_hand_edited_cross_owner_group() -> Result<()> {
    let conn = setup_conn();
    insert_pref(
        &conn,
        2200,
        STASH,
        "Preference: verify before claim",
        "Always run fresh verification before claiming completion.",
        Some("pref-a"),
        "repo",
        STASH,
    )?;
    insert_pref(
        &conn,
        2201,
        STASH,
        "Preference: fresh verification",
        "Always run fresh verification before claiming completion.",
        Some("pref-b"),
        "repo",
        STASH,
    )?;
    insert_pref(
        &conn,
        2210,
        STASH,
        "Preference: global verification",
        "Always run fresh verification before claiming completion.",
        Some("pref-c"),
        "user",
        "user:default",
    )?;
    let mut plan = build_preference_cleanup_plan(&conn, STASH)?;
    let group = plan.groups.first_mut().expect("repo duplicate group");
    group.stale_ids.push(2210);
    group.row_snapshots.push(snapshot_for_test(
        2210,
        STASH,
        "Always run fresh verification before claiming completion.",
        Some("pref-c"),
        Some("user"),
        Some("user:default"),
        None,
    ));

    let err = apply_memory_cleanup_plan(&conn, &plan).expect_err("cross-owner plan must fail");

    assert!(err.to_string().contains("owner does not match"));
    assert_active(&conn, &[2200, 2201, 2210])?;
    Ok(())
}

#[test]
fn cleanup_plan_keeps_cross_owner_preferences_separate() -> Result<()> {
    let conn = setup_conn();
    insert_pref(
        &conn,
        2200,
        STASH,
        "Preference: verify before claim",
        "Always run fresh verification before claiming completion.",
        Some("pref-a"),
        "repo",
        STASH,
    )?;
    insert_pref(
        &conn,
        2201,
        STASH,
        "Preference: fresh verification",
        "Always run fresh verification before claiming completion.",
        Some("pref-b"),
        "repo",
        STASH,
    )?;
    insert_pref(
        &conn,
        2210,
        STASH,
        "Preference: global verification",
        "Always run fresh verification before claiming completion.",
        Some("pref-c"),
        "user",
        "user:default",
    )?;

    let plan = build_preference_cleanup_plan(&conn, STASH)?;

    assert_eq!(plan.groups.len(), 1);
    assert_eq!(plan.groups[0].current_id, 2201);
    assert_eq!(plan.groups[0].stale_ids, vec![2200]);
    assert_eq!(
        plan.groups[0].owner_key.as_deref(),
        Some(STASH),
        "global preference must not be merged into repo cleanup"
    );
    Ok(())
}

fn snapshot_for_test(
    id: i64,
    project: &str,
    content: &str,
    topic_key: Option<&str>,
    owner_scope: Option<&str>,
    owner_key: Option<&str>,
    target_project: Option<&str>,
) -> MemoryCleanupRowSnapshot {
    MemoryCleanupRowSnapshot {
        id,
        project: project.to_string(),
        scope: Some("project".to_string()),
        source_project: Some(project.to_string()),
        target_project: target_project.map(str::to_string),
        status: "active".to_string(),
        content_sha256: content_sha256(content),
        updated_at_epoch: 100,
        owner_scope: owner_scope.map(str::to_string),
        owner_key: owner_key.map(str::to_string),
        memory_type: "preference".to_string(),
        topic_key: topic_key.map(str::to_string),
        state_key_id: None,
        state_key: None,
        current_memory_id: None,
    }
}

fn content_sha256(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn insert_pref(
    conn: &Connection,
    id: i64,
    project: &str,
    title: &str,
    content: &str,
    topic_key: Option<&str>,
    owner_scope: &str,
    owner_key: &str,
) -> Result<()> {
    conn.execute(
        "INSERT INTO memories
         (id, project, topic_key, title, content, memory_type, created_at_epoch,
          updated_at_epoch, status, scope, source_project, target_project, owner_scope,
          owner_key, routing_confidence, context_class)
         VALUES (?1, ?2, ?3, ?4, ?5, 'preference', 100, 100, 'active',
                 'project', ?2, ?6, ?7, ?8, 1.0, 'startup_core')",
        params![
            id,
            project,
            topic_key,
            title,
            content,
            if owner_scope == "repo" {
                Some(project)
            } else {
                None
            },
            owner_scope,
            owner_key
        ],
    )?;
    Ok(())
}

fn assert_active(conn: &Connection, ids: &[i64]) -> Result<()> {
    for id in ids {
        assert_eq!(
            conn.query_row("SELECT status FROM memories WHERE id = ?1", [id], |row| {
                row.get::<_, String>(0)
            })?,
            "active"
        );
    }
    Ok(())
}
