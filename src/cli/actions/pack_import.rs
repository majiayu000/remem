use std::{fs, path::Path};

use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection};

use super::pack_export::{
    hex_sha256, pack_memory_content_hash, PackManifest, PackMemory, PACK_FORMAT_VERSION,
};

mod active_import;

pub(in crate::cli) fn run_import_pack(pack: &Path, project: &str, dry_run: bool) -> Result<()> {
    let loaded = load_pack(pack)?;
    if dry_run {
        let db_path = crate::db::db_path();
        let conn = if db_path.exists() {
            Some(crate::db::open_db_read_only().with_context(|| {
                format!("open read-only runtime database {}", db_path.display())
            })?)
        } else {
            None
        };
        let plan = plan_loaded_pack(conn.as_ref(), project, loaded)?;
        print!("{}", render_import_plan(pack, &plan));
    } else {
        let mut conn = crate::db::open_db().context("open runtime database for pack import")?;
        let report = active_import::apply_loaded_pack(&mut conn, project, loaded)?;
        print!(
            "{}",
            active_import::render_import_apply_report(pack, &report)
        );
    }
    Ok(())
}

#[cfg(test)]
struct PackImportRequest<'a> {
    pack: &'a Path,
    target_project: &'a str,
}

#[derive(Debug, Clone, PartialEq)]
struct PackImportPlan {
    content_digest: String,
    local_store_inspected: bool,
    stats: PackImportStats,
    entries: Vec<PackImportEntry>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct PackImportStats {
    add: usize,
    dedup: usize,
    skip: usize,
    conflict: usize,
    quarantine: usize,
}

impl PackImportStats {
    fn record(&mut self, category: PackImportCategory) {
        match category {
            PackImportCategory::Add => self.add += 1,
            PackImportCategory::Dedup => self.dedup += 1,
            PackImportCategory::Skip => self.skip += 1,
            PackImportCategory::Conflict => self.conflict += 1,
            PackImportCategory::Quarantine => self.quarantine += 1,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct PackImportEntry {
    category: PackImportCategory,
    reason: String,
    title: String,
    state_key: Option<String>,
    content_hash: String,
    memory: PackMemory,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PackImportCategory {
    Add,
    Dedup,
    Skip,
    Conflict,
    Quarantine,
}

impl PackImportCategory {
    fn as_str(self) -> &'static str {
        match self {
            PackImportCategory::Add => "add",
            PackImportCategory::Dedup => "dedup",
            PackImportCategory::Skip => "skip",
            PackImportCategory::Conflict => "conflict",
            PackImportCategory::Quarantine => "quarantine",
        }
    }
}

fn render_import_plan(pack: &Path, plan: &PackImportPlan) -> String {
    let mut output = format!(
        "Pack import dry-run for {}: add={} dedup={} skip={} conflict={} quarantine={} ({} rows, digest {}).\n",
        pack.display(),
        plan.stats.add,
        plan.stats.dedup,
        plan.stats.skip,
        plan.stats.conflict,
        plan.stats.quarantine,
        plan.entries.len(),
        plan.content_digest
    );
    if !plan.local_store_inspected {
        output.push_str(
            "Local runtime database not found; planner treated the local store as empty without creating it.\n",
        );
    }
    for entry in &plan.entries {
        output.push_str(&format!(
            "- {} state_key={} hash={} title=\"{}\" reason=\"{}\"\n",
            entry.category.as_str(),
            entry.state_key.as_deref().unwrap_or("-"),
            entry.content_hash,
            single_line(&entry.title),
            single_line(&entry.reason)
        ));
    }
    output
}

fn single_line(value: &str) -> String {
    let collapsed = value.replace(['\r', '\n'], " ");
    if collapsed.len() <= 160 {
        return collapsed;
    }
    let mut end = 160;
    while end > 0 && !collapsed.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}...", &collapsed[..end])
}

#[cfg(test)]
fn plan_import_pack(conn: &Connection, request: PackImportRequest<'_>) -> Result<PackImportPlan> {
    let loaded = load_pack(request.pack)?;
    plan_loaded_pack(Some(conn), request.target_project, loaded)
}

fn plan_loaded_pack(
    conn: Option<&Connection>,
    target_project: &str,
    loaded: LoadedPack,
) -> Result<PackImportPlan> {
    let mut stats = PackImportStats::default();
    let mut entries = Vec::with_capacity(loaded.memories.len());

    for memory in loaded.memories {
        let entry = classify_pack_memory(conn, target_project, memory)?;
        stats.record(entry.category);
        entries.push(entry);
    }

    Ok(PackImportPlan {
        content_digest: loaded.manifest.content_digest,
        local_store_inspected: conn.is_some(),
        stats,
        entries,
    })
}

struct LoadedPack {
    manifest: PackManifest,
    memories: Vec<PackMemory>,
}

fn load_pack(pack: &Path) -> Result<LoadedPack> {
    let manifest_path = pack.join("pack.json");
    let memories_path = pack.join("memories.jsonl");
    let manifest_text = fs::read_to_string(&manifest_path)
        .with_context(|| format!("read {}", manifest_path.display()))?;
    let manifest: PackManifest = serde_json::from_str(&manifest_text)
        .with_context(|| format!("parse {}", manifest_path.display()))?;
    if manifest.format_version != PACK_FORMAT_VERSION {
        bail!(
            "unsupported remem pack format_version {}; supported format_version is {}",
            manifest.format_version,
            PACK_FORMAT_VERSION
        );
    }

    let memories_jsonl = fs::read_to_string(&memories_path)
        .with_context(|| format!("read {}", memories_path.display()))?;
    let content_digest = hex_sha256(memories_jsonl.as_bytes());
    if content_digest != manifest.content_digest {
        bail!(
            "pack content digest mismatch: manifest={} actual={}",
            manifest.content_digest,
            content_digest
        );
    }

    let mut memories = Vec::new();
    for (index, line) in memories_jsonl.lines().enumerate() {
        if line.trim().is_empty() {
            bail!("memories.jsonl line {} is blank", index + 1);
        }
        let memory: PackMemory = serde_json::from_str(line)
            .with_context(|| format!("parse memories.jsonl line {}", index + 1))?;
        validate_pack_memory(index + 1, &memory)?;
        memories.push(memory);
    }
    if memories.len() != manifest.memory_count {
        bail!(
            "pack memory_count mismatch: manifest={} actual={}",
            manifest.memory_count,
            memories.len()
        );
    }

    Ok(LoadedPack { manifest, memories })
}

fn validate_pack_memory(line_number: usize, memory: &PackMemory) -> Result<()> {
    if crate::memory::MemoryType::parse(&memory.memory_type).is_none() {
        bail!(
            "memories.jsonl line {} has unsupported memory_type '{}'",
            line_number,
            memory.memory_type
        );
    }
    if memory.scope != "project" {
        bail!(
            "memories.jsonl line {} has unsupported scope '{}'; project packs only accept project scope",
            line_number,
            memory.scope
        );
    }
    if memory.owner_intent != "repo" {
        bail!(
            "memories.jsonl line {} has unsupported owner_intent '{}'; project packs only accept repo-owned memories",
            line_number,
            memory.owner_intent
        );
    }
    let expected_hash = pack_memory_content_hash(
        &memory.memory_type,
        memory.state_key.as_deref(),
        &memory.title,
        &memory.content,
    );
    if memory.content_hash != expected_hash {
        bail!(
            "memories.jsonl line {} content_hash mismatch: row={} actual={}",
            line_number,
            memory.content_hash,
            expected_hash
        );
    }
    Ok(())
}

fn classify_pack_memory(
    conn: Option<&Connection>,
    target_project: &str,
    memory: PackMemory,
) -> Result<PackImportEntry> {
    let category;
    let reason;
    let local_matches = conn
        .map(|conn| load_local_matches(conn, target_project, &memory))
        .transpose()?
        .unwrap_or_default();

    if let Some(suppression) = conn
        .map(|conn| matching_suppression(conn, &memory, &local_matches))
        .transpose()?
        .flatten()
    {
        category = PackImportCategory::Skip;
        reason = format!("suppressed by local {}", suppression.label());
    } else {
        if let Some(inactive) = local_matches.iter().find(|row| !row.is_current()) {
            category = PackImportCategory::Skip;
            reason = format!(
                "inactive local identity id={} status={}",
                inactive.id, inactive.status
            );
        } else if local_matches
            .iter()
            .any(|row| row.content_hash(&memory.memory_type) == memory.content_hash)
        {
            category = PackImportCategory::Dedup;
            reason = "identical active local memory".to_string();
        } else if memory.state_key.is_some()
            && local_matches.iter().any(LocalMemoryMatch::is_current)
        {
            category = PackImportCategory::Conflict;
            reason = "active local state-key memory differs; local wins".to_string();
        } else if let Some(pattern) = crate::memory::poisoning::scan_instruction_pattern(&format!(
            "{}\n{}",
            memory.title, memory.content
        )) {
            category = PackImportCategory::Quarantine;
            reason = format!(
                "instruction pattern {}@v{}",
                pattern.pattern_id, pattern.pattern_set_version
            );
        } else {
            category = PackImportCategory::Add;
            reason = "safe to add in active import".to_string();
        }
    }

    Ok(PackImportEntry {
        category,
        reason,
        title: memory.title.clone(),
        state_key: memory.state_key.clone(),
        content_hash: memory.content_hash.clone(),
        memory,
    })
}

#[derive(Debug, Clone)]
struct SuppressionMatch {
    target_kind: String,
    target_id: Option<i64>,
    target_value: Option<String>,
}

impl SuppressionMatch {
    fn label(&self) -> String {
        match (self.target_id, self.target_value.as_deref()) {
            (Some(id), _) => format!("{}:{id}", self.target_kind),
            (None, Some(value)) => format!("{}:{value}", self.target_kind),
            (None, None) => self.target_kind.clone(),
        }
    }
}

fn matching_suppression(
    conn: &Connection,
    memory: &PackMemory,
    local_matches: &[LocalMemoryMatch],
) -> Result<Option<SuppressionMatch>> {
    let title_lower = memory.title.to_lowercase();
    let content_lower = memory.content.to_lowercase();
    let entity_names = crate::retrieval::entity::extract_entities(&memory.title, &memory.content)
        .into_iter()
        .map(|entity| entity.to_lowercase())
        .collect::<Vec<_>>();
    let mut stmt = conn.prepare(
        "SELECT target_kind, target_id, target_value
         FROM memory_suppressions
         WHERE status = 'active'
         ORDER BY updated_at_epoch DESC, id DESC",
    )?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let suppression = SuppressionMatch {
            target_kind: row.get(0)?,
            target_id: row.get(1)?,
            target_value: row.get(2)?,
        };
        let matched = match suppression.target_kind.as_str() {
            "memory" => suppression
                .target_id
                .is_some_and(|id| local_matches.iter().any(|local| local.id == id)),
            "topic_key" => suppression
                .target_value
                .as_deref()
                .is_some_and(|value| memory.state_key.as_deref() == Some(value)),
            "entity" => suppression.target_value.as_deref().is_some_and(|value| {
                let value = value.to_lowercase();
                entity_names.iter().any(|entity| entity == &value)
            }),
            "pattern" => suppression.target_value.as_deref().is_some_and(|value| {
                let value = value.to_lowercase();
                title_lower.contains(&value) || content_lower.contains(&value)
            }),
            _ => false,
        };
        if matched {
            return Ok(Some(suppression));
        }
    }
    Ok(None)
}

#[derive(Debug, Clone)]
struct LocalMemoryMatch {
    id: i64,
    title: String,
    content: String,
    status: String,
    expires_at_epoch: Option<i64>,
    state_key: Option<String>,
}

impl LocalMemoryMatch {
    fn is_current(&self) -> bool {
        self.status == "active"
            && self
                .expires_at_epoch
                .is_none_or(|expires| expires > chrono::Utc::now().timestamp())
    }

    fn content_hash(&self, memory_type: &str) -> String {
        pack_memory_content_hash(
            memory_type,
            self.state_key.as_deref(),
            &self.title,
            &self.content,
        )
    }
}

fn load_local_matches(
    conn: &Connection,
    target_project: &str,
    memory: &PackMemory,
) -> Result<Vec<LocalMemoryMatch>> {
    if let Some(state_key) = memory.state_key.as_deref() {
        load_local_state_key_matches(conn, target_project, memory, state_key)
    } else {
        load_local_content_hash_matches(conn, target_project, memory)
    }
}

fn load_local_state_key_matches(
    conn: &Connection,
    target_project: &str,
    memory: &PackMemory,
    state_key: &str,
) -> Result<Vec<LocalMemoryMatch>> {
    let mut stmt = conn.prepare(
        "SELECT m.id, m.title, m.content, m.status, m.expires_at_epoch, sk.state_key
         FROM memory_state_keys sk
         JOIN memories m ON m.state_key_id = sk.id
         WHERE sk.owner_scope = 'repo'
           AND sk.owner_key = ?1
           AND sk.memory_type = ?2
           AND sk.state_key = ?3
         ORDER BY m.updated_at_epoch DESC, m.id DESC",
    )?;
    let rows = stmt.query_map(
        params![target_project, memory.memory_type, state_key],
        local_memory_match_from_row,
    )?;
    crate::db::query::collect_rows(rows)
}

fn load_local_content_hash_matches(
    conn: &Connection,
    target_project: &str,
    memory: &PackMemory,
) -> Result<Vec<LocalMemoryMatch>> {
    let mut stmt = conn.prepare(
        "SELECT m.id, m.title, m.content, m.status, m.expires_at_epoch, sk.state_key
         FROM memories m
         LEFT JOIN memory_state_keys sk ON sk.id = m.state_key_id
         WHERE m.owner_scope = 'repo'
           AND m.owner_key = ?1
           AND m.memory_type = ?2
         ORDER BY m.updated_at_epoch DESC, m.id DESC",
    )?;
    let rows = stmt.query_map(
        params![target_project, memory.memory_type],
        local_memory_match_from_row,
    )?;
    let matches = crate::db::query::collect_rows(rows)?
        .into_iter()
        .filter(|row| row.content_hash(&memory.memory_type) == memory.content_hash)
        .collect();
    Ok(matches)
}

fn local_memory_match_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<LocalMemoryMatch> {
    Ok(LocalMemoryMatch {
        id: row.get(0)?,
        title: row.get(1)?,
        content: row.get(2)?,
        status: row.get(3)?,
        expires_at_epoch: row.get(4)?,
        state_key: row.get(5)?,
    })
}

#[cfg(test)]
mod tests;
