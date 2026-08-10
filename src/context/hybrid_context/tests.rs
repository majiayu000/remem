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
