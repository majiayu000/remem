use anyhow::{bail, Context, Result};
use chrono::Local;
use serde::Serialize;
use std::io::Read;
use std::path::Path;

use super::admin::{backup_db, default_backup_path};
use super::encrypt_state::{
    initialize_missing_database_with_key, inspect_existing_key_database, ExistingKeyDatabaseState,
};
use crate::cli::types::MemoryGovernanceCliAction;
use crate::{db, memory};

pub(in crate::cli) async fn run_dream(
    project: Option<&str>,
    profile: Option<&str>,
    dry_run: bool,
) -> Result<()> {
    let cwd = crate::cli::cwd::resolve_cwd_arg(None);
    let project = project
        .map(str::to_owned)
        .unwrap_or_else(|| db::project_from_cwd(&cwd));

    if dry_run {
        let plan = crate::dream::list_cluster_plan(&project)?;
        println!(
            "project={} clusters={} suppressed={} (dry-run, no changes)",
            project,
            plan.eligible.len(),
            plan.suppressed
        );
        for (i, c) in plan.eligible.iter().enumerate() {
            println!("  cluster[{}] size={}", i, c.members.len());
            for m in &c.members {
                println!("    id={} title={}", m.id, m.title);
            }
        }
        return Ok(());
    }

    crate::dream::process_dream_job_with_profile(&project, profile).await?;
    println!("dream complete for project={}", project);
    Ok(())
}

pub(in crate::cli) fn run_encrypt(rekey_raw: bool) -> Result<()> {
    if rekey_raw {
        return run_rekey_raw();
    }

    let key_path = db::data_dir().join(".key");
    if key_path.exists() {
        match inspect_existing_key_database(&key_path, &db::db_path())? {
            ExistingKeyDatabaseState::Encrypted => {
                println!(
                    "Database is already encrypted (verified with key file at {})",
                    key_path.display()
                );
            }
            ExistingKeyDatabaseState::Missing(key) => {
                initialize_missing_database_with_key(&db::db_path(), &key).with_context(|| {
                    format!(
                        "initialize encrypted database with existing SQLCipher key {}",
                        key_path.display()
                    )
                })?;
                println!(
                    "Initialized encrypted database at {}",
                    db::db_path().display()
                );
            }
        }
        return Ok(());
    }

    println!("Generating encryption key...");
    let key = db::generate_cipher_key()?;
    let cipher_key = db::CipherKey::Raw(key);
    println!("Key saved to {}", key_path.display());

    println!("Encrypting database (this may take a moment)...");
    let db_path = db::db_path();
    let encrypted_path = db_path.with_extension("db.enc");
    let backup_path = db_path.with_extension("db.bak");
    let encrypted_existed = encrypted_path.exists();
    let backup_existed = backup_path.exists();
    if db_path.exists() {
        if let Err(error) = db::encrypt_database(&cipher_key) {
            db::rollback_generated_key_after_encrypt_failure(
                &key_path,
                &cipher_key,
                &db_path,
                encrypted_existed,
                backup_existed,
            )
            .with_context(|| {
                format!("rollback generated key after failed database encryption: {error:#}")
            })?;
            return Err(error);
        }
    } else {
        if let Err(error) = db::open_db() {
            db::rollback_generated_key_after_encrypt_failure(
                &key_path,
                &cipher_key,
                &db_path,
                encrypted_existed,
                backup_existed,
            )
            .with_context(|| {
                format!("rollback generated key after failed database initialization: {error:#}")
            })?;
            return Err(error);
        }
        println!("Initialized encrypted database at {}", db_path.display());
    }

    println!("Done. Database is now encrypted with SQLCipher.");
    Ok(())
}

fn run_rekey_raw() -> Result<()> {
    let db_path = db::db_path();
    if !db_path.exists() {
        anyhow::bail!("database not found: {}", db_path.display());
    }

    let key_path = db::data_dir().join(".key");
    if !key_path.exists() {
        anyhow::bail!(
            "SQLCipher key file not found at {}; run `remem encrypt` first",
            key_path.display()
        );
    }
    let key_text = std::fs::read_to_string(&key_path)
        .with_context(|| format!("read SQLCipher key file {}", key_path.display()))?;
    let Some(key) = db::parse_cipher_key(&key_text)
        .with_context(|| format!("parse SQLCipher key file {}", key_path.display()))?
    else {
        anyhow::bail!("SQLCipher key file is empty: {}", key_path.display());
    };

    let db::CipherKey::Passphrase(passphrase) = key else {
        println!(
            "SQLCipher key file is already raw-key format: {}",
            key_path.display()
        );
        return Ok(());
    };
    let raw_hex = db::legacy_passphrase_to_raw_hex(&passphrase)
        .context("legacy key must be the 64-hex key generated by remem")?
        .to_string();

    let db_backup_path = default_backup_path(Local::now());
    println!("Backing up database to {}...", db_backup_path.display());
    backup_db(&db_path, &db_backup_path)?;

    let key_backup_path = db::backup_cipher_key_file(&key_path)?;
    println!("Backed up key file to {}.", key_backup_path.display());

    println!("Rekeying database to SQLCipher raw-key format...");
    {
        let legacy_key = db::CipherKey::Passphrase(passphrase);
        let conn = db::open_configured_existing_read_write_connection(&db_path, Some(&legacy_key))?;
        db::rekey_connection_to_raw(&conn, &raw_hex)?;
    }

    {
        let raw_key = db::CipherKey::Raw(raw_hex.clone());
        let conn = db::open_configured_existing_read_write_connection(&db_path, Some(&raw_key))
            .context("verify raw-key database connection after rekey")?;
        if !db::can_read_schema(&conn) {
            anyhow::bail!("raw-key database verification failed after rekey");
        }
    }

    db::write_raw_key_file(&key_path, &raw_hex).with_context(|| {
        format!(
            "database rekey succeeded but key-file rewrite failed; DB backup is {}, key backup is {}; manually prefix the existing 64-hex key with `v2:` to recover",
            db_backup_path.display(),
            key_backup_path.display()
        )
    })?;

    println!("Done. Database now uses SQLCipher raw-key format.");
    println!("Database backup: {}", db_backup_path.display());
    println!("Key backup: {}", key_backup_path.display());
    Ok(())
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct CleanupRetentionDays {
    old_events: i64,
    compressed_source_observations: i64,
    stale_memories: i64,
    archived_failures: i64,
    workstream_auto_pause: i64,
    workstream_auto_abandon: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct CleanupPlan {
    expired_memories_to_stale: usize,
    inactive_workstreams_to_pause: usize,
    long_paused_workstreams_to_abandon: usize,
    old_events_to_delete: usize,
    compressed_source_observations_to_delete: usize,
    stale_memories_to_archive: usize,
    archived_failures_to_purge: db::ArchivedFailurePurgePlan,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct CleanupApplied {
    expired_memories_marked_stale: usize,
    inactive_workstreams_paused: usize,
    long_paused_workstreams_abandoned: usize,
    old_events_deleted: usize,
    compressed_source_observations_deleted: usize,
    stale_memories_archived: usize,
    archived_failures_purged: db::ArchivedFailurePurgePlan,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct CleanupReport {
    dry_run: bool,
    retention_days: CleanupRetentionDays,
    plan: CleanupPlan,
    applied: Option<CleanupApplied>,
}

pub(in crate::cli) fn run_cleanup(
    dry_run: bool,
    json: bool,
    archived_failures: Option<i64>,
) -> Result<()> {
    let conn = db::open_db()?;
    let now_epoch = chrono::Utc::now().timestamp();
    let report = build_cleanup_report(&conn, now_epoch, dry_run, archived_failures)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }

    if dry_run {
        println!("Cleanup dry-run:");
        print_cleanup_plan(&report.plan);
        println!("  No changes written.");
    } else {
        println!("Cleanup complete:");
        print_cleanup_plan(&report.plan);
        if let Some(applied) = report.applied {
            println!("Applied:");
            println!(
                "  Expired memories marked stale: {}",
                applied.expired_memories_marked_stale
            );
            println!(
                "  Inactive workstreams paused: {}",
                applied.inactive_workstreams_paused
            );
            println!(
                "  Long-paused workstreams abandoned: {}",
                applied.long_paused_workstreams_abandoned
            );
            println!("  Old events deleted: {}", applied.old_events_deleted);
            println!(
                "  Compressed source observations deleted: {}",
                applied.compressed_source_observations_deleted
            );
            println!(
                "  Stale memories archived: {}",
                applied.stale_memories_archived
            );
            println!(
                "  Archived failures purged: pending={} extraction_tasks={} replay_ranges={} jobs={}",
                applied.archived_failures_purged.pending_observations,
                applied.archived_failures_purged.extraction_tasks,
                applied.archived_failures_purged.extraction_replay_ranges,
                applied.archived_failures_purged.jobs
            );
        }
    }
    Ok(())
}

fn build_cleanup_report(
    conn: &rusqlite::Connection,
    now_epoch: i64,
    dry_run: bool,
    archived_failure_days: Option<i64>,
) -> Result<CleanupReport> {
    let purge_archived_failures = archived_failure_days.is_some();
    let archived_failure_days = archived_failure_days.unwrap_or(db::ARCHIVED_FAILURE_PURGE_DAYS);
    let retention_days = CleanupRetentionDays {
        old_events: memory::OLD_EVENT_RETENTION_DAYS,
        compressed_source_observations: memory::COMPRESSED_SOURCE_OBSERVATION_RETENTION_DAYS,
        stale_memories: memory::STALE_MEMORY_ARCHIVE_DAYS,
        archived_failures: archived_failure_days,
        workstream_auto_pause: crate::workstream::DEFAULT_AUTO_PAUSE_DAYS,
        workstream_auto_abandon: crate::workstream::DEFAULT_AUTO_ABANDON_DAYS,
    };
    let plan = build_cleanup_plan(
        conn,
        now_epoch,
        purge_archived_failures.then_some(archived_failure_days),
    )?;
    let applied = if dry_run {
        None
    } else {
        Some(apply_cleanup_plan(
            conn,
            now_epoch,
            purge_archived_failures.then_some(archived_failure_days),
        )?)
    };
    Ok(CleanupReport {
        dry_run,
        retention_days,
        plan,
        applied,
    })
}

fn build_cleanup_plan(
    conn: &rusqlite::Connection,
    now_epoch: i64,
    archived_failure_days: Option<i64>,
) -> Result<CleanupPlan> {
    Ok(CleanupPlan {
        expired_memories_to_stale: memory::lifecycle::count_expired_active_memories(
            conn, now_epoch,
        )?,
        inactive_workstreams_to_pause: crate::workstream::count_auto_pause_all_inactive_at(
            conn,
            now_epoch,
            crate::workstream::DEFAULT_AUTO_PAUSE_DAYS,
        )?,
        long_paused_workstreams_to_abandon: crate::workstream::count_auto_abandon_all_inactive_at(
            conn,
            now_epoch,
            crate::workstream::DEFAULT_AUTO_ABANDON_DAYS,
        )?,
        old_events_to_delete: memory::count_old_events_at(
            conn,
            now_epoch,
            memory::OLD_EVENT_RETENTION_DAYS,
        )?,
        compressed_source_observations_to_delete:
            memory::count_compressed_source_observations_to_delete_at(
                conn,
                now_epoch,
                memory::COMPRESSED_SOURCE_OBSERVATION_RETENTION_DAYS,
            )?,
        stale_memories_to_archive: memory::count_stale_memories_to_archive_at(
            conn,
            now_epoch,
            memory::STALE_MEMORY_ARCHIVE_DAYS,
        )?,
        archived_failures_to_purge: match archived_failure_days {
            Some(days) => db::count_archived_failures_to_purge_at(conn, now_epoch, days)?,
            None => db::ArchivedFailurePurgePlan::default(),
        },
    })
}

fn apply_cleanup_plan(
    conn: &rusqlite::Connection,
    now_epoch: i64,
    archived_failure_days: Option<i64>,
) -> Result<CleanupApplied> {
    Ok(CleanupApplied {
        expired_memories_marked_stale: memory::lifecycle::expire_active_memories(conn, now_epoch)?,
        inactive_workstreams_paused: crate::workstream::auto_pause_all_inactive_at(
            conn,
            now_epoch,
            crate::workstream::DEFAULT_AUTO_PAUSE_DAYS,
        )?,
        long_paused_workstreams_abandoned: crate::workstream::auto_abandon_all_inactive_at(
            conn,
            now_epoch,
            crate::workstream::DEFAULT_AUTO_ABANDON_DAYS,
        )?,
        old_events_deleted: memory::cleanup_old_events_at(
            conn,
            now_epoch,
            memory::OLD_EVENT_RETENTION_DAYS,
        )?,
        compressed_source_observations_deleted: memory::cleanup_compressed_source_observations_at(
            conn,
            now_epoch,
            memory::COMPRESSED_SOURCE_OBSERVATION_RETENTION_DAYS,
        )?,
        stale_memories_archived: memory::archive_stale_memories_at(
            conn,
            now_epoch,
            memory::STALE_MEMORY_ARCHIVE_DAYS,
        )?,
        archived_failures_purged: match archived_failure_days {
            Some(days) => db::purge_archived_failures_at(conn, now_epoch, days)?,
            None => db::ArchivedFailurePurgePlan::default(),
        },
    })
}

fn print_cleanup_plan(plan: &CleanupPlan) {
    println!(
        "  Expired memories to mark stale: {}",
        plan.expired_memories_to_stale
    );
    println!(
        "  Inactive workstreams to pause: {}",
        plan.inactive_workstreams_to_pause
    );
    println!(
        "  Long-paused workstreams to abandon: {}",
        plan.long_paused_workstreams_to_abandon
    );
    println!(
        "  Old events to delete (>30 days): {}",
        plan.old_events_to_delete
    );
    println!(
        "  Compressed source observations to delete (>90 days after compression): {}",
        plan.compressed_source_observations_to_delete
    );
    println!(
        "  Stale memories to archive (>180 days): {}",
        plan.stale_memories_to_archive
    );
    println!(
        "  Archived failures to purge: pending={} extraction_tasks={} replay_ranges={} jobs={}",
        plan.archived_failures_to_purge.pending_observations,
        plan.archived_failures_to_purge.extraction_tasks,
        plan.archived_failures_to_purge.extraction_replay_ranges,
        plan.archived_failures_to_purge.jobs
    );
}

pub(in crate::cli) struct GovernanceCliRequest<'a> {
    pub(in crate::cli) project: Option<&'a str>,
    pub(in crate::cli) action: MemoryGovernanceCliAction,
    pub(in crate::cli) acknowledge_pattern: Option<&'a str>,
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
    pub(in crate::cli) json: bool,
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
        MemoryGovernanceCliAction::AcknowledgePattern => {
            memory::governance::MemoryGovernanceAction::AcknowledgePattern
        }
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
            if req.json {
                let output = memory::governance::GovernMemoryResult {
                    dry_run,
                    action: action.as_str().to_string(),
                    reason: req.reason.map(str::to_string),
                    affected: Vec::new(),
                };
                println!("{}", serde_json::to_string_pretty(&output)?);
                return Ok(());
            }
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
    if dry_run && !req.dry_run && !req.confirm_destructive && !req.json {
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
            acknowledge_pattern: req.acknowledge_pattern,
        },
    )?;
    if req.json {
        println!("{}", serde_json::to_string_pretty(&result)?);
        return Ok(());
    }
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
mod tests;
