use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};

pub(super) fn backfill_content_identity_hashes(conn: &Connection) -> Result<()> {
    let raw_updates =
        backfill_raw_messages(conn).context("backfill raw_messages content hashes")?;
    let blob_updates = backfill_event_blobs(conn).context("backfill event_blobs content hashes")?;
    let capture_updates =
        backfill_captured_events(conn).context("backfill captured_events content hashes")?;
    crate::log::info(
        "migrate",
        &format!(
            "backfilled content identity hashes: raw_messages={raw_updates} event_blobs={blob_updates} captured_events={capture_updates}"
        ),
    );
    Ok(())
}

fn backfill_raw_messages(conn: &Connection) -> Result<usize> {
    let ids = row_ids(conn, "raw_messages")?;
    let mut changed = 0;

    for id in ids {
        let Some((project, session_id, role, content, current_hash)) = conn
            .query_row(
                "SELECT project, session_id, role, content, content_hash
                 FROM raw_messages WHERE id = ?1",
                params![id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                    ))
                },
            )
            .optional()?
        else {
            continue;
        };
        let next_hash = crate::db::content_identity_hash(content.as_bytes());
        if current_hash == next_hash {
            continue;
        }

        if let Some((existing_id, existing_content)) = conn
            .query_row(
                "SELECT id, content FROM raw_messages
                 WHERE project = ?1 AND session_id = ?2 AND role = ?3
                   AND content_hash = ?4 AND id != ?5",
                params![project, session_id, role, next_hash, id],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?
        {
            if existing_content != content {
                bail!(
                    "sha256 content identity collision in raw_messages: ids {id} and {existing_id}"
                );
            }
            if existing_id < id {
                conn.execute("DELETE FROM raw_messages WHERE id = ?1", params![id])?;
            } else {
                conn.execute(
                    "DELETE FROM raw_messages WHERE id = ?1",
                    params![existing_id],
                )?;
                conn.execute(
                    "UPDATE raw_messages SET content_hash = ?1 WHERE id = ?2",
                    params![next_hash, id],
                )?;
            }
            changed += 1;
            continue;
        }

        conn.execute(
            "UPDATE raw_messages SET content_hash = ?1 WHERE id = ?2",
            params![next_hash, id],
        )?;
        changed += 1;
    }

    Ok(changed)
}

fn backfill_event_blobs(conn: &Connection) -> Result<usize> {
    let ids = row_ids(conn, "event_blobs")?;
    let mut changed = 0;

    for id in ids {
        let Some((current_hash, encoding, bytes)) = conn
            .query_row(
                "SELECT content_hash, content_encoding, content_bytes
                 FROM event_blobs WHERE id = ?1",
                params![id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Vec<u8>>(2)?,
                    ))
                },
            )
            .optional()?
        else {
            continue;
        };
        let next_hash = crate::db::content_identity_hash(&bytes);
        if current_hash == next_hash {
            continue;
        }

        if let Some((existing_id, existing_encoding, existing_bytes)) = conn
            .query_row(
                "SELECT id, content_encoding, content_bytes
                 FROM event_blobs WHERE content_hash = ?1 AND id != ?2",
                params![next_hash, id],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Vec<u8>>(2)?,
                    ))
                },
            )
            .optional()?
        {
            if existing_encoding != encoding || existing_bytes != bytes {
                bail!(
                    "sha256 content identity collision in event_blobs: ids {id} and {existing_id}"
                );
            }
            if existing_id < id {
                conn.execute(
                    "UPDATE captured_events SET content_blob_id = ?1 WHERE content_blob_id = ?2",
                    params![existing_id, id],
                )?;
                conn.execute("DELETE FROM event_blobs WHERE id = ?1", params![id])?;
            } else {
                conn.execute(
                    "UPDATE captured_events SET content_blob_id = ?1 WHERE content_blob_id = ?2",
                    params![id, existing_id],
                )?;
                conn.execute(
                    "DELETE FROM event_blobs WHERE id = ?1",
                    params![existing_id],
                )?;
                conn.execute(
                    "UPDATE event_blobs SET content_hash = ?1 WHERE id = ?2",
                    params![next_hash, id],
                )?;
            }
            changed += 1;
            continue;
        }

        conn.execute(
            "UPDATE event_blobs SET content_hash = ?1 WHERE id = ?2",
            params![next_hash, id],
        )?;
        changed += 1;
    }

    Ok(changed)
}

fn backfill_captured_events(conn: &Connection) -> Result<usize> {
    let ids = row_ids(conn, "captured_events")?;
    let mut changed = 0;

    for id in ids {
        let (current_hash, content_text, content_blob_id): (String, Option<String>, Option<i64>) =
            conn.query_row(
                "SELECT content_hash, content_text, content_blob_id
                 FROM captured_events WHERE id = ?1",
                params![id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )?;
        let content_bytes = if let Some(blob_id) = content_blob_id {
            conn.query_row(
                "SELECT content_bytes FROM event_blobs WHERE id = ?1",
                params![blob_id],
                |row| row.get::<_, Vec<u8>>(0),
            )
            .with_context(|| {
                format!("captured_events.id={id} references missing event_blobs.id={blob_id}")
            })?
        } else if let Some(content) = content_text {
            content.into_bytes()
        } else {
            bail!("captured_events.id={id} has neither content_text nor content_blob_id");
        };
        let next_hash = crate::db::content_identity_hash(&content_bytes);
        if current_hash == next_hash {
            continue;
        }

        conn.execute(
            "UPDATE captured_events SET content_hash = ?1 WHERE id = ?2",
            params![next_hash, id],
        )?;
        changed += 1;
    }

    Ok(changed)
}

fn row_ids(conn: &Connection, table: &str) -> Result<Vec<i64>> {
    let sql = match table {
        "raw_messages" => "SELECT id FROM raw_messages ORDER BY id",
        "event_blobs" => "SELECT id FROM event_blobs ORDER BY id",
        "captured_events" => "SELECT id FROM captured_events ORDER BY id",
        _ => bail!("unsupported content identity backfill table: {table}"),
    };
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map([], |row| row.get::<_, i64>(0))?;
    let mut ids = Vec::new();
    for row in rows {
        ids.push(row?);
    }
    Ok(ids)
}
