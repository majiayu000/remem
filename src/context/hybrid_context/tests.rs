use super::*;

#[test]
fn final_id_filter_keeps_the_current_state_key_memory_without_an_alias() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    crate::migrate::run_migrations(&conn)?;
    let project = "/tmp/remem-hybrid-state-key";
    let memory_id = 41;
    crate::context::tests::insert_memory(
        &conn,
        memory_id,
        project,
        Some("hybrid-state-key"),
        "decision",
        "Use qualified state-key filters",
        "The current state-key memory must survive the final fused-ID recheck.",
        1_710_000_000,
    );
    conn.execute(
        "UPDATE memory_state_keys SET current_memory_id = ?1 WHERE state_key = ?2",
        rusqlite::params![memory_id, format!("context-fixture-{memory_id}")],
    )?;

    let memories = query_owner_included_memories_by_ids(&conn, project, &[memory_id], None, &[])?;

    assert_eq!(
        memories.iter().map(|memory| memory.id).collect::<Vec<_>>(),
        vec![memory_id]
    );
    Ok(())
}

#[test]
fn context_filters_keep_owner_rows_and_reach_them_through_project_indexes() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    crate::migrate::run_migrations(&conn)?;
    let project = "/tmp/remem-hybrid-owner-index";
    let other = "/tmp/remem-hybrid-owner-other";
    // (id, owner_scope, owner_key, target_project, project, scope)
    let rows: [(i64, Option<&str>, Option<&str>, Option<&str>, &str, &str); 6] = [
        (51, Some("repo"), Some(project), None, other, "project"),
        (
            52,
            Some("repo"),
            Some(other),
            Some(project),
            other,
            "project",
        ),
        (53, None, None, None, project, "project"),
        (54, None, None, None, project, "global"),
        (
            55,
            Some("repo"),
            Some(other),
            Some(other),
            project,
            "project",
        ),
        (56, None, None, Some(project), other, "project"),
    ];
    for (id, owner_scope, owner_key, target_project, row_project, scope) in rows {
        crate::context::tests::insert_memory(
            &conn,
            id,
            row_project,
            None,
            "decision",
            "Owner index fixture",
            "Rows reached by owner, target or legacy project.",
            1_710_000_000 + id,
        );
        conn.execute(
            "UPDATE memories
             SET owner_scope = ?2, owner_key = ?3, target_project = ?4, project = ?5, scope = ?6
             WHERE id = ?1",
            rusqlite::params![
                id,
                owner_scope,
                owner_key,
                target_project,
                row_project,
                scope
            ],
        )?;
    }

    let mut idx = 1;
    let mut conditions = Vec::new();
    let mut params = Vec::new();
    push_context_memory_filters(
        &conn,
        project,
        None,
        &[],
        "m",
        &mut idx,
        &mut conditions,
        &mut params,
    )?;
    let sql = format!(
        "SELECT m.id FROM memories m WHERE {} ORDER BY m.id",
        conditions.join(" AND ")
    );
    let refs = crate::db::to_sql_refs(&params);
    let mut stmt = conn.prepare(&sql)?;
    let ids = stmt
        .query_map(refs.as_slice(), |row| row.get::<_, i64>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    assert_eq!(ids, vec![51, 52, 53]);

    let mut plan = conn.prepare(&format!("EXPLAIN QUERY PLAN {sql}"))?;
    let details = plan
        .query_map(refs.as_slice(), |row| row.get::<_, String>(3))?
        .collect::<rusqlite::Result<Vec<_>>>()?
        .join("\n");
    // Candidate ids come from the three project-bearing indexes; the outer
    // memories scan only tests rowids against that set and reads matching rows.
    for access in [
        "idx_memories_owner_status (owner_scope=? AND owner_key=?)",
        "idx_memories_target_project_status (target_project=?)",
        "(project=?)",
    ] {
        assert!(
            details.contains(access),
            "missing {access} in plan:\n{details}"
        );
    }
    Ok(())
}
