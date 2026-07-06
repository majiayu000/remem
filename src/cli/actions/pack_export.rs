use std::{fs, path::Path};

use anyhow::{Context, Result};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub(super) const PACK_FORMAT_VERSION: u32 = 1;
const DEFAULT_LIMIT: i64 = 10_000;
const MAX_LIMIT: i64 = 100_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::cli) struct PackExportStats {
    pub exported: usize,
    pub output: std::path::PathBuf,
    pub digest: String,
}

pub(in crate::cli) fn run_export_pack(output: &Path, project: &str, limit: i64) -> Result<()> {
    let conn = crate::db::open_db().context("open runtime database for pack export")?;
    let stats = export_pack(
        &conn,
        PackExportRequest {
            output,
            project,
            limit,
        },
    )?;
    println!(
        "Exported {} project memories to {} as remem pack {}.",
        stats.exported,
        stats.output.display(),
        stats.digest
    );
    Ok(())
}

struct PackExportRequest<'a> {
    output: &'a Path,
    project: &'a str,
    limit: i64,
}

fn export_pack(conn: &Connection, request: PackExportRequest<'_>) -> Result<PackExportStats> {
    let rows = load_pack_memories(conn, request.project, normalize_limit(request.limit))?;
    let mut pack_rows = rows
        .into_iter()
        .map(PackMemory::try_from)
        .collect::<Result<Vec<_>>>()?;
    pack_rows.sort_by(|left, right| {
        (
            left.memory_type.as_str(),
            left.state_key.as_deref().unwrap_or(""),
            left.content_hash.as_str(),
        )
            .cmp(&(
                right.memory_type.as_str(),
                right.state_key.as_deref().unwrap_or(""),
                right.content_hash.as_str(),
            ))
    });

    let memories_jsonl = render_memories_jsonl(&pack_rows)?;
    let content_digest = hex_sha256(memories_jsonl.as_bytes());
    let manifest = PackManifest {
        format_version: PACK_FORMAT_VERSION,
        project: request.project.to_string(),
        exporter: "remem".to_string(),
        exporter_version: env!("CARGO_PKG_VERSION").to_string(),
        memory_count: pack_rows.len(),
        content_digest: content_digest.clone(),
    };

    fs::create_dir_all(request.output)
        .with_context(|| format!("create pack directory {}", request.output.display()))?;
    fs::write(
        request.output.join("memories.jsonl"),
        memories_jsonl.as_bytes(),
    )
    .with_context(|| format!("write {}", request.output.join("memories.jsonl").display()))?;
    fs::write(
        request.output.join("pack.json"),
        format!("{}\n", serde_json::to_string_pretty(&manifest)?),
    )
    .with_context(|| format!("write {}", request.output.join("pack.json").display()))?;
    fs::write(
        request.output.join("INDEX.md"),
        render_index(request.project, &pack_rows).as_bytes(),
    )
    .with_context(|| format!("write {}", request.output.join("INDEX.md").display()))?;

    Ok(PackExportStats {
        exported: pack_rows.len(),
        output: request.output.to_path_buf(),
        digest: content_digest,
    })
}

fn normalize_limit(limit: i64) -> i64 {
    if limit <= 0 {
        DEFAULT_LIMIT
    } else {
        limit.min(MAX_LIMIT)
    }
}

fn load_pack_memories(conn: &Connection, project: &str, limit: i64) -> Result<Vec<PackMemoryRow>> {
    let conditions = [
        "m.owner_scope = 'repo'".to_string(),
        "m.owner_key = ?1".to_string(),
        "COALESCE(m.target_project, m.project) = ?1".to_string(),
        "COALESCE(m.scope, 'project') = 'project'".to_string(),
        "COALESCE(m.context_class, 'startup_core') = 'startup_core'".to_string(),
        crate::memory::memory_current_filter_sql("m.status", "m.expires_at_epoch", false),
        crate::memory::suppression::memory_policy_filter_sql("m"),
    ];
    let sql = format!(
        "SELECT m.id, m.title, m.content, m.memory_type, COALESCE(m.scope, 'project'),
                sk.state_key, m.confidence, m.created_at_epoch,
                m.valid_from_epoch, m.expires_at_epoch,
                m.owner_scope, m.owner_key
         FROM memories m
         LEFT JOIN memory_state_keys sk ON sk.id = m.state_key_id
         WHERE {}
         ORDER BY m.memory_type ASC, COALESCE(sk.state_key, '') ASC, m.id ASC
         LIMIT ?2",
        conditions.join(" AND ")
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params![project, limit], |row| {
        Ok(PackMemoryRow {
            id: row.get(0)?,
            title: row.get(1)?,
            content: row.get(2)?,
            memory_type: row.get(3)?,
            scope: row.get(4)?,
            state_key: row.get(5)?,
            confidence: row.get(6)?,
            created_at_epoch: row.get(7)?,
            valid_from_epoch: row.get(8)?,
            expires_at_epoch: row.get(9)?,
            owner_scope: row.get(10)?,
            owner_key: row.get(11)?,
        })
    })?;
    crate::db::query::collect_rows(rows)
}

#[derive(Debug, Clone)]
struct PackMemoryRow {
    id: i64,
    title: String,
    content: String,
    memory_type: String,
    scope: String,
    state_key: Option<String>,
    confidence: Option<f64>,
    created_at_epoch: i64,
    valid_from_epoch: Option<i64>,
    expires_at_epoch: Option<i64>,
    owner_scope: String,
    owner_key: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(super) struct PackMemory {
    pub(super) title: String,
    pub(super) content: String,
    pub(super) memory_type: String,
    pub(super) scope: String,
    pub(super) state_key: Option<String>,
    pub(super) state_key_confidence: Option<f64>,
    pub(super) state_key_reason: Option<String>,
    pub(super) confidence: Option<f64>,
    pub(super) created_at_epoch: i64,
    pub(super) valid_from_epoch: Option<i64>,
    pub(super) expires_at_epoch: Option<i64>,
    pub(super) owner_intent: String,
    pub(super) origin: String,
    pub(super) content_hash: String,
}

impl TryFrom<PackMemoryRow> for PackMemory {
    type Error = anyhow::Error;

    fn try_from(row: PackMemoryRow) -> Result<Self> {
        ensure_no_redaction_hit(row.id, "title", &row.title)?;
        ensure_no_redaction_hit(row.id, "content", &row.content)?;
        let content_hash = pack_memory_content_hash(
            &row.memory_type,
            row.state_key.as_deref(),
            &row.title,
            &row.content,
        );
        Ok(Self {
            title: row.title,
            content: row.content,
            memory_type: row.memory_type,
            scope: row.scope,
            state_key: row.state_key,
            state_key_confidence: None,
            state_key_reason: None,
            confidence: row.confidence,
            created_at_epoch: row.created_at_epoch,
            valid_from_epoch: row.valid_from_epoch,
            expires_at_epoch: row.expires_at_epoch,
            owner_intent: row.owner_scope,
            origin: format!("repo:{}", row.owner_key),
            content_hash,
        })
    }
}

fn ensure_no_redaction_hit(memory_id: i64, field: &str, value: &str) -> Result<()> {
    let redacted = crate::adapter::redaction::redact_sensitive_text(value);
    if redacted != value {
        anyhow::bail!(
            "pack export blocked by redaction scan for memory id={} field={}",
            memory_id,
            field
        );
    }
    Ok(())
}

pub(super) fn render_memories_jsonl(rows: &[PackMemory]) -> Result<String> {
    let mut output = String::new();
    for row in rows {
        output.push_str(&serde_json::to_string(row)?);
        output.push('\n');
    }
    Ok(output)
}

fn render_index(project: &str, rows: &[PackMemory]) -> String {
    let mut output = format!("# remem Project Memory Pack\n\nProject: `{project}`\n\n");
    let mut current_type = "";
    for row in rows {
        if row.memory_type != current_type {
            current_type = &row.memory_type;
            output.push_str(&format!("## {}\n\n", row.memory_type));
        }
        output.push_str(&format!("- {} `{}`\n", row.title, row.content_hash));
    }
    output
}

#[derive(Debug, Serialize, Deserialize)]
pub(super) struct PackManifest {
    pub(super) format_version: u32,
    pub(super) project: String,
    pub(super) exporter: String,
    pub(super) exporter_version: String,
    pub(super) memory_count: usize,
    pub(super) content_digest: String,
}

pub(super) fn hex_sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>()
}

pub(super) fn pack_memory_content_hash(
    memory_type: &str,
    state_key: Option<&str>,
    title: &str,
    content: &str,
) -> String {
    hex_sha256(
        format!(
            "{}\0{}\0{}\0{}",
            memory_type,
            state_key.unwrap_or(""),
            title,
            content
        )
        .as_bytes(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn pack_export_is_deterministic_and_filters_project_repo_startup_memories() -> Result<()> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
        crate::migrate::run_migrations(&conn)?;
        insert_memory(
            &conn,
            1,
            "/repo",
            "decision",
            "Deployment",
            "Use blue-green deploys",
        )?;
        insert_memory(
            &conn,
            2,
            "/repo",
            "bugfix",
            "Retry bug",
            "Backoff must be bounded",
        )?;
        insert_memory(&conn, 3, "/other", "decision", "Other", "Other project")?;
        conn.execute(
            "UPDATE memories SET owner_scope = 'user', owner_key = 'user:default' WHERE id = 3",
            [],
        )?;

        let dir = unique_pack_dir("pack-export-deterministic");
        let _ = fs::remove_dir_all(&dir);
        let stats = export_pack(
            &conn,
            PackExportRequest {
                output: &dir,
                project: "/repo",
                limit: 100,
            },
        )?;
        let first_jsonl = fs::read_to_string(dir.join("memories.jsonl"))?;
        let first_manifest = fs::read_to_string(dir.join("pack.json"))?;
        let first_index = fs::read_to_string(dir.join("INDEX.md"))?;

        let stats_again = export_pack(
            &conn,
            PackExportRequest {
                output: &dir,
                project: "/repo",
                limit: 100,
            },
        )?;

        assert_eq!(stats.exported, 2);
        assert_eq!(stats.digest, stats_again.digest);
        assert_eq!(first_jsonl, fs::read_to_string(dir.join("memories.jsonl"))?);
        assert_eq!(first_manifest, fs::read_to_string(dir.join("pack.json"))?);
        assert_eq!(first_index, fs::read_to_string(dir.join("INDEX.md"))?);
        assert!(first_jsonl.contains("\"owner_intent\":\"repo\""));
        assert!(!first_jsonl.contains("\"id\":"));
        assert!(!first_jsonl.contains("Other project"));
        let _ = fs::remove_dir_all(&dir);
        Ok(())
    }

    #[test]
    fn pack_export_redaction_hit_blocks_export_without_silent_skip() -> Result<()> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
        crate::migrate::run_migrations(&conn)?;
        insert_memory(
            &conn,
            1,
            "/repo",
            "decision",
            "Secret",
            "Use token sk-proj-12345678 only locally",
        )?;
        let dir = unique_pack_dir("pack-export-redaction");
        let _ = fs::remove_dir_all(&dir);

        let error = export_pack(
            &conn,
            PackExportRequest {
                output: &dir,
                project: "/repo",
                limit: 100,
            },
        )
        .expect_err("secret-like content should block pack export");

        assert!(
            error
                .to_string()
                .contains("pack export blocked by redaction scan for memory id=1"),
            "{error:?}"
        );
        assert!(!dir.join("memories.jsonl").exists());
        let _ = fs::remove_dir_all(&dir);
        Ok(())
    }

    fn unique_pack_dir(label: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        std::env::temp_dir().join(format!("remem-{label}-{}-{nanos}", std::process::id()))
    }

    fn insert_memory(
        conn: &Connection,
        id: i64,
        project: &str,
        memory_type: &str,
        title: &str,
        content: &str,
    ) -> Result<()> {
        conn.execute(
            "INSERT INTO memories
             (id, project, title, content, memory_type, created_at_epoch,
              updated_at_epoch, status, scope, source_project, target_project,
              owner_scope, owner_key, context_class)
             VALUES (?1, ?2, ?3, ?4, ?5, ?1, ?1, 'active', 'project',
                     ?2, ?2, 'repo', ?2, 'startup_core')",
            rusqlite::params![id, project, title, content, memory_type],
        )?;
        Ok(())
    }
}
