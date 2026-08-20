use anyhow::Result;
use rusqlite::{Connection, OptionalExtension};

fn finding(findings: &mut Vec<String>, detail: impl std::fmt::Display) {
    findings.push(format!(
        "v084_session_observatory critical shape mismatch: {detail}"
    ));
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
    for index in indexes {
        let (name, unique, partial) = index?;
        if unique && !partial && index_columns(conn, &name)? == expected {
            return Ok(());
        }
    }
    finding(
        findings,
        format!("{table} missing UNIQUE({})", expected.join(",")),
    );
    Ok(())
}

pub(super) fn require_foreign_key(
    conn: &Connection,
    findings: &mut Vec<String>,
    table: &str,
    from: &str,
    target: &str,
    to: &str,
    on_delete: &str,
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
    for row in rows {
        let (actual_target, actual_from, actual_to, actual_delete) = row?;
        if actual_target == target
            && actual_from == from
            && actual_to == to
            && actual_delete.eq_ignore_ascii_case(on_delete)
        {
            return Ok(());
        }
    }
    finding(
        findings,
        format!("{table}.{from} missing REFERENCES {target}({to}) ON DELETE {on_delete}"),
    );
    Ok(())
}

pub(super) fn require_create_sql(
    conn: &Connection,
    findings: &mut Vec<String>,
    object_type: &str,
    name: &str,
    expected: &str,
) -> Result<()> {
    let actual = conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type = ?1 AND name = ?2",
            [object_type, name],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    let Some(actual) = actual else {
        return Ok(());
    };
    if normalize_sql(&actual) != normalize_sql(expected) {
        finding(
            findings,
            format!("{object_type} {name} SQL differs from the canonical v084 contract"),
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

fn normalize_sql(sql: &str) -> String {
    sql.trim_end_matches(';')
        .to_ascii_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}
