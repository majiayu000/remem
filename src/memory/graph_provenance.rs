use anyhow::{bail, Result};
use rusqlite::{Connection, OptionalExtension};

pub(crate) fn validate_trusted_source_candidate(
    conn: &Connection,
    source_candidate_id: Option<i64>,
    source_operation_id: Option<i64>,
) -> Result<()> {
    let candidate_id = source_candidate_id
        .ok_or_else(|| anyhow::anyhow!("trusted graph edge requires source candidate id"))?;
    let operation_id = source_operation_id
        .ok_or_else(|| anyhow::anyhow!("trusted graph edge requires source operation id"))?;
    let operation = conn
        .query_row(
            "SELECT source, source_candidate_id FROM memory_operation_log WHERE id = ?1",
            [operation_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<i64>>(1)?)),
        )
        .optional()?;

    let Some((source, operation_candidate_id)) = operation else {
        bail!("trusted graph edge source operation id {operation_id} does not exist");
    };
    if operation_candidate_id != Some(candidate_id) {
        bail!(
            "trusted graph edge source operation {operation_id} references candidate {:?}, not {candidate_id}",
            operation_candidate_id
        );
    }

    match source.as_str() {
        "memory_candidate" => {
            ensure_memory_candidate_exists(conn, candidate_id)?;
        }
        "graph_candidate" => {
            ensure_graph_candidate_exists(conn, candidate_id)?;
        }
        other => bail!(
            "trusted graph edge source operation {operation_id} uses unsupported candidate source {other}"
        ),
    }

    Ok(())
}

fn ensure_memory_candidate_exists(conn: &Connection, candidate_id: i64) -> Result<()> {
    let exists: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM memory_candidates WHERE id = ?1)",
        [candidate_id],
        |row| row.get(0),
    )?;
    if !exists {
        bail!("trusted graph edge source memory candidate id {candidate_id} does not exist");
    }
    Ok(())
}

fn ensure_graph_candidate_exists(conn: &Connection, candidate_id: i64) -> Result<()> {
    let exists: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM graph_candidates WHERE id = ?1)",
        [candidate_id],
        |row| row.get(0),
    )?;
    if !exists {
        bail!("trusted graph edge source graph candidate id {candidate_id} does not exist");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_conn() -> Result<Connection> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(
            "CREATE TABLE memory_candidates (id INTEGER PRIMARY KEY);
             CREATE TABLE graph_candidates (id INTEGER PRIMARY KEY);
             CREATE TABLE memory_operation_log (
                 id INTEGER PRIMARY KEY,
                 source TEXT NOT NULL,
                 source_candidate_id INTEGER
             );",
        )?;
        Ok(conn)
    }

    #[test]
    fn trusted_source_candidate_requires_operation_candidate_match() -> Result<()> {
        let conn = test_conn()?;
        conn.execute("INSERT INTO memory_candidates(id) VALUES (1), (2)", [])?;
        conn.execute(
            "INSERT INTO memory_operation_log(id, source, source_candidate_id)
             VALUES (10, 'memory_candidate', 2)",
            [],
        )?;

        let err = validate_trusted_source_candidate(&conn, Some(1), Some(10)).expect_err(
            "trusted graph provenance must bind operation and edge to the same candidate",
        );
        assert!(err
            .to_string()
            .contains("references candidate Some(2), not 1"));
        Ok(())
    }
}
