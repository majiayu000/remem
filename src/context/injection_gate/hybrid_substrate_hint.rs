use anyhow::Result;
use rusqlite::Connection;
use sha2::{Digest, Sha256};

const SUBSTRATE_SCAN_LIMIT: i64 = 200;

pub(super) fn compute_hybrid_substrate_fingerprint(
    conn: &Connection,
    project: &str,
) -> Result<String> {
    let mut version = SubstrateVersionBuilder::new();
    push_fts_signal(conn, project, &mut version)?;
    push_entity_signal(conn, project, &mut version)?;
    push_embedding_signal(conn, project, &mut version)?;
    Ok(version.finish())
}

fn push_fts_signal(
    conn: &Connection,
    project: &str,
    version: &mut SubstrateVersionBuilder,
) -> Result<()> {
    if !crate::retrieval::temporal::sqlite_table_exists(conn, "memories_fts")? {
        version.push("memories_fts_table", "missing");
        return Ok(());
    }
    version.push("memories_fts_table", "present");
    let has_search_context = column_exists(conn, "memories_fts", "search_context")?;
    version.push(
        "memories_fts_search_context",
        if has_search_context { "1" } else { "0" },
    );
    let search_context_expr = if has_search_context {
        "f.search_context"
    } else {
        "''"
    };
    let summary_sql = format!(
        "SELECT COUNT(*), MAX(f.rowid),
                COALESCE(SUM(length(f.title) + length(f.content) + length({search_context_expr})), 0)
         FROM memories_fts f
         JOIN memories m ON m.id = f.rowid
         WHERE m.project = ?1"
    );
    let summary = conn.query_row(&summary_sql, [project], |row| {
        Ok(vec![
            row.get::<_, i64>(0)?.to_string(),
            row.get::<_, Option<i64>>(1)?
                .unwrap_or_default()
                .to_string(),
            row.get::<_, i64>(2)?.to_string(),
        ])
    })?;
    version.push_row("memories_fts_summary", &summary);

    let row_sql = format!(
        "SELECT f.rowid, f.title, f.content, {search_context_expr}
         FROM memories_fts f
         JOIN memories m ON m.id = f.rowid
         WHERE m.project = ?1
         ORDER BY m.updated_at_epoch DESC, f.rowid DESC
         LIMIT ?2"
    );
    let mut stmt = conn.prepare(&row_sql)?;
    let rows = stmt.query_map(rusqlite::params![project, SUBSTRATE_SCAN_LIMIT], |row| {
        Ok(vec![
            row.get::<_, i64>(0)?.to_string(),
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
        ])
    })?;
    push_rows("memories_fts", rows, version)
}

fn push_entity_signal(
    conn: &Connection,
    project: &str,
    version: &mut SubstrateVersionBuilder,
) -> Result<()> {
    let has_entities = crate::retrieval::temporal::sqlite_table_exists(conn, "entities")?;
    let has_memory_entities =
        crate::retrieval::temporal::sqlite_table_exists(conn, "memory_entities")?;
    version.push(
        "entities_table",
        if has_entities { "present" } else { "missing" },
    );
    version.push(
        "memory_entities_table",
        if has_memory_entities {
            "present"
        } else {
            "missing"
        },
    );
    if !has_entities || !has_memory_entities {
        return Ok(());
    }

    let summary = conn.query_row(
        "SELECT COUNT(*), MAX(me.memory_id), MAX(me.entity_id)
         FROM memory_entities me
         JOIN memories m ON m.id = me.memory_id
         WHERE m.project = ?1",
        [project],
        |row| {
            Ok(vec![
                row.get::<_, i64>(0)?.to_string(),
                row.get::<_, Option<i64>>(1)?
                    .unwrap_or_default()
                    .to_string(),
                row.get::<_, Option<i64>>(2)?
                    .unwrap_or_default()
                    .to_string(),
            ])
        },
    )?;
    version.push_row("memory_entities_summary", &summary);

    let mut stmt = conn.prepare(
        "SELECT me.memory_id, me.entity_id, e.canonical_name,
                e.entity_type, e.mention_count
         FROM memory_entities me
         JOIN entities e ON e.id = me.entity_id
         JOIN memories m ON m.id = me.memory_id
         WHERE m.project = ?1
         ORDER BY m.updated_at_epoch DESC, me.memory_id DESC, me.entity_id DESC
         LIMIT ?2",
    )?;
    let rows = stmt.query_map(rusqlite::params![project, SUBSTRATE_SCAN_LIMIT], |row| {
        Ok(vec![
            row.get::<_, i64>(0)?.to_string(),
            row.get::<_, i64>(1)?.to_string(),
            row.get::<_, String>(2)?,
            row.get::<_, Option<String>>(3)?.unwrap_or_default(),
            row.get::<_, Option<i64>>(4)?
                .unwrap_or_default()
                .to_string(),
        ])
    })?;
    push_rows("memory_entity", rows, version)
}

fn push_embedding_signal(
    conn: &Connection,
    project: &str,
    version: &mut SubstrateVersionBuilder,
) -> Result<()> {
    if !crate::retrieval::temporal::sqlite_table_exists(conn, "memory_embeddings")? {
        version.push("memory_embeddings_table", "missing");
        return Ok(());
    }
    version.push("memory_embeddings_table", "present");
    let summary = conn.query_row(
        "SELECT COUNT(*), MAX(e.memory_id), MAX(e.updated_at_epoch)
         FROM memory_embeddings e
         JOIN memories m ON m.id = e.memory_id
         WHERE m.project = ?1",
        [project],
        |row| {
            Ok(vec![
                row.get::<_, i64>(0)?.to_string(),
                row.get::<_, Option<i64>>(1)?
                    .unwrap_or_default()
                    .to_string(),
                row.get::<_, Option<i64>>(2)?
                    .unwrap_or_default()
                    .to_string(),
            ])
        },
    )?;
    version.push_row("memory_embeddings_summary", &summary);

    let mut stmt = conn.prepare(
        "SELECT e.memory_id, e.dimensions, e.model, e.content_hash,
                e.updated_at_epoch, length(e.embedding), hex(substr(e.embedding, 1, 32))
         FROM memory_embeddings e
         JOIN memories m ON m.id = e.memory_id
         WHERE m.project = ?1
         ORDER BY e.updated_at_epoch DESC, e.memory_id DESC
         LIMIT ?2",
    )?;
    let rows = stmt.query_map(rusqlite::params![project, SUBSTRATE_SCAN_LIMIT], |row| {
        Ok(vec![
            row.get::<_, i64>(0)?.to_string(),
            row.get::<_, i64>(1)?.to_string(),
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, i64>(4)?.to_string(),
            row.get::<_, i64>(5)?.to_string(),
            row.get::<_, String>(6)?,
        ])
    })?;
    push_rows("memory_embedding", rows, version)
}

fn push_rows<I>(label: &'static str, rows: I, version: &mut SubstrateVersionBuilder) -> Result<()>
where
    I: Iterator<Item = rusqlite::Result<Vec<String>>>,
{
    let mut count = 0usize;
    for row in rows {
        version.push_row(label, &row?);
        count += 1;
    }
    version.push(&format!("{label}_count"), &count.to_string());
    Ok(())
}

fn column_exists(conn: &Connection, table: &str, column: &str) -> Result<bool> {
    let mut stmt = conn.prepare("SELECT name FROM pragma_table_info(?1)")?;
    let rows = stmt.query_map([table], |row| row.get::<_, String>(0))?;
    for row in rows {
        if row? == column {
            return Ok(true);
        }
    }
    Ok(false)
}

struct SubstrateVersionBuilder {
    hasher: Sha256,
}

impl SubstrateVersionBuilder {
    fn new() -> Self {
        Self {
            hasher: Sha256::new(),
        }
    }

    fn push(&mut self, key: &str, value: &str) {
        self.hasher.update(key.as_bytes());
        self.hasher.update([0]);
        self.hasher.update(value.as_bytes());
        self.hasher.update([0xff]);
    }

    fn push_row(&mut self, label: &str, fields: &[String]) {
        self.push(label, &fields.len().to_string());
        for field in fields {
            self.push("field", field);
        }
        self.push("row_end", label);
    }

    fn finish(self) -> String {
        format!("{:x}", self.hasher.finalize())
    }
}
