use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::io::Read;
use std::path::Path;

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
    if db::db_path().exists() {
        db::encrypt_database(&key)?;
    } else {
        let _conn = db::open_db()?;
        println!(
            "Initialized encrypted database at {}",
            db::db_path().display()
        );
    }

    println!("Done. Database is now encrypted with SQLCipher.");
    if db::db_path().with_extension("db.bak").exists() {
        println!("Backup saved as remem.db.bak");
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct CleanupRetentionDays {
    old_events: i64,
    compressed_source_observations: i64,
    stale_memories: i64,
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
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct CleanupApplied {
    expired_memories_marked_stale: usize,
    inactive_workstreams_paused: usize,
    long_paused_workstreams_abandoned: usize,
    old_events_deleted: usize,
    compressed_source_observations_deleted: usize,
    stale_memories_archived: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct CleanupReport {
    dry_run: bool,
    retention_days: CleanupRetentionDays,
    plan: CleanupPlan,
    applied: Option<CleanupApplied>,
}

pub(in crate::cli) fn run_cleanup(dry_run: bool, json: bool) -> Result<()> {
    let conn = db::open_db()?;
    let now_epoch = chrono::Utc::now().timestamp();
    let report = build_cleanup_report(&conn, now_epoch, dry_run)?;
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
        }
    }
    Ok(())
}

fn build_cleanup_report(
    conn: &rusqlite::Connection,
    now_epoch: i64,
    dry_run: bool,
) -> Result<CleanupReport> {
    let retention_days = CleanupRetentionDays {
        old_events: memory::OLD_EVENT_RETENTION_DAYS,
        compressed_source_observations: memory::COMPRESSED_SOURCE_OBSERVATION_RETENTION_DAYS,
        stale_memories: memory::STALE_MEMORY_ARCHIVE_DAYS,
        workstream_auto_pause: crate::workstream::DEFAULT_AUTO_PAUSE_DAYS,
        workstream_auto_abandon: crate::workstream::DEFAULT_AUTO_ABANDON_DAYS,
    };
    let plan = build_cleanup_plan(conn, now_epoch)?;
    let applied = if dry_run {
        None
    } else {
        Some(apply_cleanup_plan(conn, now_epoch)?)
    };
    Ok(CleanupReport {
        dry_run,
        retention_days,
        plan,
        applied,
    })
}

fn build_cleanup_plan(conn: &rusqlite::Connection, now_epoch: i64) -> Result<CleanupPlan> {
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
    })
}

fn apply_cleanup_plan(conn: &rusqlite::Connection, now_epoch: i64) -> Result<CleanupApplied> {
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
mod tests {
    use serde_json::Value;

    use super::*;
    use crate::db::test_support::ScopedTestDataDir;
    use crate::memory::governance::{GovernMemoryResult, GovernedMemory};

    #[test]
    fn parse_governance_id_text_accepts_commas_and_whitespace() -> Result<()> {
        let ids = parse_governance_id_text("1, 2\n3\t4")?;
        assert_eq!(ids, vec![1, 2, 3, 4]);
        Ok(())
    }

    #[test]
    fn run_encrypt_initializes_missing_database() -> Result<()> {
        let test_dir = ScopedTestDataDir::new("encrypt-empty");
        std::env::remove_var("REMEM_ALLOW_PLAINTEXT_DB");

        run_encrypt()?;

        assert!(test_dir.path.join(".key").exists());
        assert!(test_dir.db_path().exists());
        let header = std::fs::read(test_dir.db_path())?;
        assert_ne!(&header[..16], b"SQLite format 3\0");

        let conn = rusqlite::Connection::open(test_dir.db_path())?;
        crate::db::apply_cipher_key_if_available(&conn)?;
        let count: i64 =
            conn.query_row("SELECT COUNT(*) FROM sqlite_master", [], |row| row.get(0))?;
        assert!(count > 0);
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

    #[test]
    fn cli_governance_json_result_is_machine_parseable(
    ) -> std::result::Result<(), serde_json::Error> {
        let result = GovernMemoryResult {
            dry_run: true,
            action: "stale".to_string(),
            reason: Some("stale fact".to_string()),
            affected: vec![GovernedMemory {
                id: 7,
                title: "Old memory".to_string(),
                previous_status: "active".to_string(),
                new_status: "stale".to_string(),
            }],
        };

        let text = serde_json::to_string(&result)?;
        let parsed: Value = serde_json::from_str(&text)?;

        assert_eq!(parsed["dry_run"], true);
        assert_eq!(parsed["action"], "stale");
        assert_eq!(parsed["affected"][0]["new_status"], "stale");
        Ok(())
    }

    #[test]
    fn cleanup_report_json_exposes_dry_run_plan_counts(
    ) -> std::result::Result<(), serde_json::Error> {
        let report = CleanupReport {
            dry_run: true,
            retention_days: CleanupRetentionDays {
                old_events: 30,
                compressed_source_observations: 90,
                stale_memories: 180,
                workstream_auto_pause: 14,
                workstream_auto_abandon: 30,
            },
            plan: CleanupPlan {
                expired_memories_to_stale: 1,
                inactive_workstreams_to_pause: 2,
                long_paused_workstreams_to_abandon: 3,
                old_events_to_delete: 4,
                compressed_source_observations_to_delete: 5,
                stale_memories_to_archive: 6,
            },
            applied: None,
        };

        let parsed = serde_json::to_value(report)?;

        assert_eq!(parsed["dry_run"], true);
        assert_eq!(parsed["applied"], Value::Null);
        assert_eq!(parsed["plan"]["old_events_to_delete"], 4);
        assert_eq!(
            parsed["plan"]["compressed_source_observations_to_delete"],
            5
        );
        Ok(())
    }
}
