use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use std::collections::HashSet;

pub fn link_entities(conn: &Connection, memory_id: i64, entities: &[String]) -> Result<()> {
    let entity_names = unique_entity_names(entities);
    if entity_names.is_empty() {
        return Ok(());
    }

    with_entity_savepoint(conn, "link", || {
        let mut affected_entity_ids = Vec::new();
        for name in entity_names {
            let entity_id = ensure_entity(conn, &name)?;
            conn.execute(
                "INSERT OR IGNORE INTO memory_entities (memory_id, entity_id) VALUES (?1, ?2)",
                params![memory_id, entity_id],
            )?;
            affected_entity_ids.push(entity_id);
        }
        refresh_mention_counts(conn, &affected_entity_ids)
    })
}

pub fn refresh_memory_entities(
    conn: &Connection,
    memory_id: i64,
    entities: &[String],
) -> Result<()> {
    let entity_names = unique_entity_names(entities);

    with_entity_savepoint(conn, "refresh", || {
        let mut affected_entity_ids = entity_ids_for_memory(conn, memory_id)?;
        conn.execute(
            "DELETE FROM memory_entities WHERE memory_id = ?1",
            params![memory_id],
        )?;

        for name in entity_names {
            let entity_id = ensure_entity(conn, &name)?;
            conn.execute(
                "INSERT OR IGNORE INTO memory_entities (memory_id, entity_id) VALUES (?1, ?2)",
                params![memory_id, entity_id],
            )?;
            affected_entity_ids.push(entity_id);
        }

        refresh_mention_counts(conn, &affected_entity_ids)
    })
}

fn unique_entity_names(entities: &[String]) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut names = Vec::new();

    for entity in entities {
        let name = entity.trim();
        if name.is_empty() {
            continue;
        }
        if seen.insert(name.to_lowercase()) {
            names.push(name.to_string());
        }
    }

    names
}

fn ensure_entity(conn: &Connection, name: &str) -> Result<i64> {
    conn.execute(
        "INSERT INTO entities (canonical_name, entity_type, mention_count)
         VALUES (?1, NULL, 0)
         ON CONFLICT(canonical_name) DO NOTHING",
        params![name],
    )?;
    conn.query_row(
        "SELECT id FROM entities WHERE canonical_name = ?1 COLLATE NOCASE",
        params![name],
        |row| row.get(0),
    )
    .with_context(|| format!("entity id missing after upsert for {name}"))
}

fn entity_ids_for_memory(conn: &Connection, memory_id: i64) -> Result<Vec<i64>> {
    let mut stmt = conn.prepare("SELECT entity_id FROM memory_entities WHERE memory_id = ?1")?;
    let rows = stmt.query_map(params![memory_id], |row| row.get::<_, i64>(0))?;
    crate::db::query::collect_rows(rows)
}

fn refresh_mention_counts(conn: &Connection, entity_ids: &[i64]) -> Result<()> {
    let mut seen = HashSet::new();
    for entity_id in entity_ids.iter().copied() {
        if !seen.insert(entity_id) {
            continue;
        }
        conn.execute(
            "UPDATE entities
             SET mention_count = (
                 SELECT COUNT(*)
                 FROM memory_entities
                 WHERE entity_id = ?1
             )
             WHERE id = ?1",
            params![entity_id],
        )?;
    }
    Ok(())
}

fn with_entity_savepoint<T, F>(conn: &Connection, label: &str, f: F) -> Result<T>
where
    F: FnOnce() -> Result<T>,
{
    let (begin_sql, release_sql, rollback_sql) = match label {
        "refresh" => (
            "SAVEPOINT remem_entity_refresh;",
            "RELEASE SAVEPOINT remem_entity_refresh;",
            "ROLLBACK TO SAVEPOINT remem_entity_refresh;
             RELEASE SAVEPOINT remem_entity_refresh;",
        ),
        _ => (
            "SAVEPOINT remem_entity_link;",
            "RELEASE SAVEPOINT remem_entity_link;",
            "ROLLBACK TO SAVEPOINT remem_entity_link;
             RELEASE SAVEPOINT remem_entity_link;",
        ),
    };

    conn.execute_batch(begin_sql)?;
    match f() {
        Ok(value) => {
            conn.execute_batch(release_sql)?;
            Ok(value)
        }
        Err(error) => {
            if let Err(rollback_error) = conn.execute_batch(rollback_sql) {
                return Err(error.context(format!(
                    "entity {label} rollback also failed: {rollback_error}"
                )));
            }
            Err(error)
        }
    }
}
