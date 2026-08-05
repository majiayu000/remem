use anyhow::Result;
use rusqlite::{Connection, OptionalExtension};

fn finding(findings: &mut Vec<String>, detail: impl std::fmt::Display) {
    findings.push(format!(
        "v076_dream_poisoning_quarantine critical shape mismatch: {detail}"
    ));
}

pub(super) fn require_primary_key(
    conn: &Connection,
    findings: &mut Vec<String>,
    table: &str,
    column: &str,
) -> Result<()> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info(\"{table}\")"))?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(1)?, row.get::<_, i64>(5)?))
    })?;
    let mut actual = rows
        .filter_map(|row| match row {
            Ok((name, position)) if position > 0 => Some(Ok((position, name))),
            Ok(_) => None,
            Err(error) => Some(Err(error)),
        })
        .collect::<std::result::Result<Vec<_>, _>>()?;
    actual.sort_unstable();
    if actual != [(1, column.to_string())] {
        finding(findings, format!("{table}.{column} must be PRIMARY KEY"));
    }
    Ok(())
}

pub(super) fn require_unique_columns(
    conn: &Connection,
    findings: &mut Vec<String>,
    table: &str,
    expected: &[&str],
) -> Result<()> {
    let mut stmt = conn.prepare(&format!("PRAGMA index_list(\"{table}\")"))?;
    let indexes = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(1)?,
            row.get::<_, i64>(2)? != 0,
            row.get::<_, i64>(4)? != 0,
        ))
    })?;
    let mut found = false;
    for index in indexes {
        let (name, unique, partial) = index?;
        if unique && !partial && index_columns(conn, &name)? == expected {
            found = true;
        }
    }
    if !found {
        finding(
            findings,
            format!("{table} missing UNIQUE({})", expected.join(",")),
        );
    }
    Ok(())
}

pub(super) fn require_restrict_foreign_key(
    conn: &Connection,
    findings: &mut Vec<String>,
    table: &str,
    from: &str,
    target: &str,
    to: &str,
) -> Result<()> {
    let mut stmt = conn.prepare(&format!("PRAGMA foreign_key_list(\"{table}\")"))?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, String>(6)?,
        ))
    })?;
    let mut found = false;
    for row in rows {
        let (actual_target, actual_from, actual_to, on_delete) = row?;
        if (
            actual_target,
            actual_from,
            actual_to,
            on_delete.to_ascii_uppercase(),
        ) == (
            target.to_string(),
            from.to_string(),
            to.to_string(),
            "RESTRICT".to_string(),
        ) {
            found = true;
        }
    }
    if !found {
        finding(
            findings,
            format!("{table}.{from} missing REFERENCES {target}({to}) ON DELETE RESTRICT"),
        );
    }
    Ok(())
}

pub(super) fn require_index_columns(
    conn: &Connection,
    findings: &mut Vec<String>,
    index: &str,
    expected: &[&str],
) -> Result<()> {
    let actual = index_columns(conn, index)?;
    if actual != expected {
        finding(
            findings,
            format!("index {index} columns {actual:?}, expected {expected:?}"),
        );
    }
    Ok(())
}

fn index_columns(conn: &Connection, index: &str) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(&format!("PRAGMA index_info(\"{index}\")"))?;
    let columns = stmt
        .query_map([], |row| row.get::<_, String>(2))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(columns)
}

pub(super) fn require_sql_fragments(
    conn: &Connection,
    findings: &mut Vec<String>,
    object_type: &str,
    name: &str,
    fragments: &[&str],
) -> Result<()> {
    let sql = conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type = ?1 AND name = ?2",
            [object_type, name],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    let Some(sql) = sql else {
        return Ok(());
    };
    let normalized = sql
        .to_ascii_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    for fragment in fragments {
        if !normalized.contains(fragment) {
            finding(
                findings,
                format!("{object_type} {name} missing SQL contract `{fragment}`"),
            );
        }
    }
    Ok(())
}
