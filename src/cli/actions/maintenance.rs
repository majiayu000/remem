use anyhow::{bail, Context, Result};
use std::io::Read;
use std::path::Path;

use crate::cli::types::MemoryGovernanceCliAction;
use crate::{db, memory};

pub(in crate::cli) async fn run_dream(project: Option<&str>, dry_run: bool) -> Result<()> {
    let cwd = crate::cli::cwd::resolve_cwd_arg(None);
    let project = project
        .map(str::to_owned)
        .unwrap_or_else(|| db::project_from_cwd(&cwd));

    if dry_run {
        let clusters = crate::dream::list_clusters(&project)?;
        println!(
            "project={} clusters={} (dry-run, no changes)",
            project,
            clusters.len()
        );
        for (i, c) in clusters.iter().enumerate() {
            println!("  cluster[{}] size={}", i, c.members.len());
            for m in &c.members {
                println!("    id={} title={}", m.id, m.title);
            }
        }
        return Ok(());
    }

    crate::dream::process_dream_job(&project).await?;
    println!("dream complete for project={}", project);
    Ok(())
}

pub(in crate::cli) fn run_encrypt() -> Result<()> {
    let key_path = db::data_dir().join(".key");
    if key_path.exists() {
        println!(
            "Database is already encrypted (key file exists at {})",
            key_path.display()
        );
        return Ok(());
    }

    println!("Generating encryption key...");
    let key = db::generate_cipher_key()?;
    println!("Key saved to {}", key_path.display());

    println!("Encrypting database (this may take a moment)...");
    db::encrypt_database(&key)?;

    println!("Done. Database is now encrypted with SQLCipher.");
    println!("Backup saved as remem.db.bak");
    Ok(())
}

pub(in crate::cli) fn run_cleanup() -> Result<()> {
    let conn = db::open_db()?;
    let events_deleted = memory::cleanup_old_events(&conn, 30)?;
    let memories_archived = memory::archive_stale_memories(&conn, 180)?;
    println!("Cleanup complete:");
    println!("  Old events deleted (>30 days): {}", events_deleted);
    println!(
        "  Stale memories archived (>180 days): {}",
        memories_archived
    );
    Ok(())
}

pub(in crate::cli) struct GovernanceCliRequest<'a> {
    pub(in crate::cli) project: Option<&'a str>,
    pub(in crate::cli) action: MemoryGovernanceCliAction,
    pub(in crate::cli) reason: Option<&'a str>,
    pub(in crate::cli) actor: Option<&'a str>,
    pub(in crate::cli) query: Option<&'a str>,
    pub(in crate::cli) memory_type: Option<&'a str>,
    pub(in crate::cli) status: Option<&'a str>,
    pub(in crate::cli) limit: i64,
    pub(in crate::cli) offset: i64,
    pub(in crate::cli) from_file: Option<&'a Path>,
    pub(in crate::cli) read_stdin: bool,
    pub(in crate::cli) confirm_destructive: bool,
    pub(in crate::cli) dry_run: bool,
    pub(in crate::cli) ids: &'a [i64],
}

pub(in crate::cli) fn run_governance(req: GovernanceCliRequest<'_>) -> Result<()> {
    let cwd = crate::cli::cwd::resolve_cwd_arg(None);
    let project = req
        .project
        .map(str::to_owned)
        .unwrap_or_else(|| db::project_from_cwd(&cwd));
    let action = match req.action {
        MemoryGovernanceCliAction::Delete => memory::governance::MemoryGovernanceAction::Delete,
        MemoryGovernanceCliAction::Reject => memory::governance::MemoryGovernanceAction::Reject,
        MemoryGovernanceCliAction::Stale => memory::governance::MemoryGovernanceAction::MarkStale,
    };
    let conn = db::open_db()?;
    let mut ids = collect_governance_ids(req.ids, req.from_file, req.read_stdin)?;
    let selector_used = has_selector(req.query, req.memory_type, req.status);
    if selector_used {
        let selected = memory::governance::select_memory_ids(
            &conn,
            &memory::governance::GovernanceSelector {
                project: &project,
                query: req.query,
                memory_type: req.memory_type,
                status: req.status,
                limit: req.limit,
                offset: req.offset,
            },
        )?;
        ids.extend(selected);
    }
    let dry_run = req.dry_run || !req.confirm_destructive;
    if ids.is_empty() {
        let input_supplied =
            selector_used || req.from_file.is_some() || req.read_stdin || !req.ids.is_empty();
        if input_supplied {
            let mode = if dry_run { "dry-run" } else { "applied" };
            println!(
                "memory governance {} action={} project={} affected=0",
                mode,
                action.as_str(),
                project
            );
            return Ok(());
        }
        bail!(
            "memory governance requires at least one memory id or selector (--query, --memory-type, --status, --from-file, --stdin)"
        );
    }
    if dry_run && !req.dry_run && !req.confirm_destructive {
        println!(
            "memory governance preview: --confirm-destructive not supplied; no changes written"
        );
    }
    let result = memory::governance::govern_memories(
        &conn,
        &memory::governance::GovernMemoryRequest {
            project: &project,
            ids: &ids,
            action,
            reason: req.reason,
            actor: req.actor,
            dry_run,
            confirm_destructive: req.confirm_destructive,
        },
    )?;
    let mode = if result.dry_run { "dry-run" } else { "applied" };
    println!(
        "memory governance {} action={} project={} affected={}",
        mode,
        result.action,
        project,
        result.affected.len()
    );
    for memory in result.affected {
        println!(
            "  id={} {} -> {} title={}",
            memory.id, memory.previous_status, memory.new_status, memory.title
        );
    }
    Ok(())
}

fn has_selector(query: Option<&str>, memory_type: Option<&str>, status: Option<&str>) -> bool {
    [query, memory_type, status]
        .into_iter()
        .flatten()
        .any(|value| !value.trim().is_empty())
}

fn collect_governance_ids(
    positional_ids: &[i64],
    from_file: Option<&Path>,
    read_stdin: bool,
) -> Result<Vec<i64>> {
    let mut ids = positional_ids.to_vec();
    if let Some(path) = from_file {
        let contents = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read memory ids from {}", path.display()))?;
        ids.extend(parse_governance_id_text(&contents)?);
    }
    if read_stdin {
        let mut contents = String::new();
        std::io::stdin()
            .read_to_string(&mut contents)
            .context("failed to read memory ids from stdin")?;
        ids.extend(parse_governance_id_text(&contents)?);
    }
    Ok(ids)
}

fn parse_governance_id_text(input: &str) -> Result<Vec<i64>> {
    let mut ids = Vec::new();
    for token in input.split(|ch: char| ch.is_whitespace() || ch == ',') {
        let token = token.trim();
        if token.is_empty() {
            continue;
        }
        let id = token
            .parse::<i64>()
            .with_context(|| format!("invalid memory id: {token}"))?;
        if id <= 0 {
            bail!("memory id must be positive: {id}");
        }
        ids.push(id);
    }
    Ok(ids)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_governance_id_text_accepts_commas_and_whitespace() -> Result<()> {
        let ids = parse_governance_id_text("1, 2\n3\t4")?;
        assert_eq!(ids, vec![1, 2, 3, 4]);
        Ok(())
    }

    #[test]
    fn parse_governance_id_text_rejects_invalid_ids() {
        let err = parse_governance_id_text("1 nope 2").expect_err("invalid id should fail");
        assert!(err.to_string().contains("invalid memory id"));
    }

    #[test]
    fn collect_governance_ids_reads_file_sources() -> Result<()> {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "remem-governance-ids-{}-{}.txt",
            std::process::id(),
            nanos
        ));
        std::fs::write(&path, "5\n6,7")?;
        let ids = collect_governance_ids(&[4], Some(&path), false)?;
        std::fs::remove_file(&path)?;
        assert_eq!(ids, vec![4, 5, 6, 7]);
        Ok(())
    }
}
