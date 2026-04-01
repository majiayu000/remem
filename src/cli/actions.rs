use anyhow::Result;

use crate::{db, memory, pending_admin, preference};

use super::{PendingAction, PreferenceAction};

fn resolve_cwd_project() -> (String, String) {
    let cwd = std::env::current_dir()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let project = db::project_from_cwd(&cwd);
    (cwd, project)
}

pub(super) fn run_preferences(action: PreferenceAction) -> Result<()> {
    let conn = db::open_db()?;
    let (_, default_project) = resolve_cwd_project();

    match action {
        PreferenceAction::List => preference::list_preferences(&conn, &default_project)?,
        PreferenceAction::Add {
            project,
            global,
            text,
        } => {
            let proj = project.unwrap_or(default_project);
            let id = preference::add_preference(&conn, &proj, &text, global)?;
            let scope_label = if global { "global" } else { "project" };
            println!(
                "Preference added (id={}, scope={}) for project '{}'",
                id, scope_label, proj
            );
        }
        PreferenceAction::Remove { id } => {
            if preference::remove_preference(&conn, id)? {
                println!("Preference {} archived.", id);
            } else {
                println!("Preference {} not found or not a preference type.", id);
            }
        }
    }

    Ok(())
}

pub(super) fn run_pending(action: PendingAction) -> Result<()> {
    let conn = db::open_db()?;

    match action {
        PendingAction::ListFailed { project, limit } => {
            let rows = pending_admin::list_failed(&conn, project.as_deref(), limit)?;
            if rows.is_empty() {
                println!("No failed pending observations.");
                return Ok(());
            }
            println!("Failed pending observations ({}):", rows.len());
            for row in rows {
                let ts = chrono::DateTime::from_timestamp(row.updated_at_epoch, 0)
                    .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
                    .unwrap_or_default();
                let err = row
                    .last_error
                    .as_deref()
                    .map(|e| db::truncate_str(e, 120).to_string())
                    .unwrap_or_default();
                println!(
                    "  [{}] {} | {} | {} | attempt={} | {}",
                    row.id, row.project, row.session_id, row.tool_name, row.attempt_count, ts
                );
                if !err.is_empty() {
                    println!("      error: {}", err);
                }
            }
        }
        PendingAction::RetryFailed { project, limit } => {
            let count = pending_admin::retry_failed(&conn, project.as_deref(), limit)?;
            println!("Moved {} failed rows back to pending.", count);
        }
        PendingAction::PurgeFailed {
            project,
            older_than_days,
        } => {
            let count = pending_admin::purge_failed(&conn, project.as_deref(), older_than_days)?;
            println!(
                "Purged {} failed rows older than {} day(s).",
                count, older_than_days
            );
        }
    }

    Ok(())
}

pub(super) fn run_status() -> Result<()> {
    let conn = db::open_db()?;
    let db_path = db::db_path();
    let db_size = std::fs::metadata(&db_path).map(|m| m.len()).unwrap_or(0);
    let version = env!("CARGO_PKG_VERSION");
    let stats = db::query_system_stats(&conn)?;

    let today_start = chrono::Local::now()
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .map(|dt| dt.and_utc().timestamp())
        .unwrap_or(0);
    let daily_stats = db::query_daily_activity_stats(&conn, today_start)?;
    let top_projects = db::query_top_projects(&conn, 5)?;

    println!("remem v{}", version);
    println!(
        "Database: {} ({:.1} MB)",
        db_path.display(),
        db_size as f64 / 1_048_576.0
    );
    println!();
    println!("  Memories:      {:>6}", stats.active_memories);
    println!("  Observations:  {:>6}", stats.active_observations);
    println!("  Sessions:      {:>6}", stats.session_summaries);
    println!("  Pending:       {:>6}", stats.pending_observations);
    println!("  Pending failed:{:>6}", stats.failed_pending_observations);
    println!();
    println!("Today:");
    println!("  New memories:      {:>4}", daily_stats.memories);
    println!("  New observations:  {:>4}", daily_stats.observations);

    if !top_projects.is_empty() {
        println!();
        println!("Top projects:");
        for project in &top_projects {
            println!("  {:>4}  {}", project.count, project.project);
        }
    }

    Ok(())
}

pub(super) fn run_search(
    query: &str,
    project: Option<&str>,
    memory_type: Option<&str>,
    limit: i64,
) -> Result<()> {
    let conn = db::open_db()?;
    let results = crate::search::search(&conn, Some(query), project, memory_type, limit, 0, false)?;

    if results.is_empty() {
        println!("No results found.");
        return Ok(());
    }

    println!("Found {} result(s):\n", results.len());
    for memory in &results {
        let date = chrono::DateTime::from_timestamp(memory.created_at_epoch, 0)
            .map(|dt| dt.format("%Y-%m-%d").to_string())
            .unwrap_or_default();
        let preview = memory
            .text
            .lines()
            .next()
            .unwrap_or("")
            .chars()
            .take(80)
            .collect::<String>();
        println!(
            "  [{}] {} | {} | {} | {}",
            memory.id, memory.memory_type, memory.project, date, memory.title
        );
        if !preview.is_empty() && preview != memory.title {
            println!("       {}", preview);
        }
    }

    Ok(())
}

pub(super) fn run_show(id: i64) -> Result<()> {
    let conn = db::open_db()?;
    let memories = memory::get_memories_by_ids(&conn, &[id], None)?;

    let Some(memory) = memories.first() else {
        println!("Memory {} not found.", id);
        return Ok(());
    };

    let created = chrono::DateTime::from_timestamp(memory.created_at_epoch, 0)
        .map(|dt| dt.format("%Y-%m-%d %H:%M UTC").to_string())
        .unwrap_or_default();
    let updated = chrono::DateTime::from_timestamp(memory.updated_at_epoch, 0)
        .map(|dt| dt.format("%Y-%m-%d %H:%M UTC").to_string())
        .unwrap_or_default();

    println!("ID:       {}", memory.id);
    println!("Title:    {}", memory.title);
    println!("Type:     {}", memory.memory_type);
    println!("Project:  {}", memory.project);
    println!("Scope:    {}", memory.scope);
    println!("Status:   {}", memory.status);
    if let Some(topic_key) = &memory.topic_key {
        println!("Topic:    {}", topic_key);
    }
    if let Some(branch) = &memory.branch {
        println!("Branch:   {}", branch);
    }
    println!("Created:  {}", created);
    println!("Updated:  {}", updated);
    println!();
    println!("{}", memory.text);

    Ok(())
}

pub(super) fn run_backfill_entities() -> Result<()> {
    let conn = db::open_db()?;
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM memories WHERE status = 'active'",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);
    println!("Backfilling entities from {} active memories...", count);

    let mut stmt =
        conn.prepare("SELECT id, title, content FROM memories WHERE status = 'active'")?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;

    let mut total_entities = 0usize;
    let mut memories_processed = 0usize;
    for row in rows {
        let (id, title, content) = row?;
        let entities = crate::entity::extract_entities(&title, &content);
        if !entities.is_empty() {
            crate::entity::link_entities(&conn, id, &entities)?;
            total_entities += entities.len();
        }
        memories_processed += 1;
        if memories_processed.is_multiple_of(100) {
            println!("  processed {}/{}", memories_processed, count);
        }
    }

    let unique: i64 = conn
        .query_row("SELECT COUNT(*) FROM entities", [], |row| row.get(0))
        .unwrap_or(0);
    println!(
        "Done. {} entities extracted, {} unique entities, {} memories processed.",
        total_entities, unique, memories_processed
    );
    Ok(())
}

pub(super) fn run_eval_local() -> Result<()> {
    let conn = db::open_db()?;
    let report = crate::eval_local::run_eval(&conn)?;
    print!("{}", report);
    Ok(())
}

pub(super) fn run_eval(dataset_path: &str, k: usize) -> Result<()> {
    #[derive(serde::Deserialize)]
    struct GoldenDataset {
        queries: Vec<GoldenQuery>,
    }

    #[derive(serde::Deserialize)]
    struct GoldenQuery {
        id: String,
        query: String,
        category: String,
        project: Option<String>,
        relevant_ids: Vec<i64>,
    }

    let content = std::fs::read_to_string(dataset_path)
        .map_err(|e| anyhow::anyhow!("cannot read {}: {}", dataset_path, e))?;
    let dataset: GoldenDataset = serde_json::from_str(&content)?;
    let conn = db::open_db()?;

    let mut total_rr = 0.0;
    let mut total_p = 0.0;
    let mut total_r = 0.0;
    let mut total_hit = 0.0;
    let mut evaluated = 0usize;

    println!("remem eval — {} queries, k={}\n", dataset.queries.len(), k);

    for query in &dataset.queries {
        let results = crate::search::search(
            &conn,
            Some(&query.query),
            query.project.as_deref(),
            None,
            k as i64,
            0,
            false,
        )?;
        let result_ids: Vec<i64> = results.iter().map(|memory| memory.id).collect();

        let rr = crate::eval_metrics::reciprocal_rank(&result_ids, &query.relevant_ids);
        let p = crate::eval_metrics::precision_at_k(&result_ids, &query.relevant_ids, k);
        let r = crate::eval_metrics::recall_at_k(&result_ids, &query.relevant_ids, k);
        let hit = crate::eval_metrics::hit_at_k(&result_ids, &query.relevant_ids, k);

        let status = if query.relevant_ids.is_empty() {
            if results.is_empty() {
                "PASS"
            } else {
                "---"
            }
        } else if hit > 0.0 {
            "HIT"
        } else {
            "MISS"
        };

        println!(
            "  [{}] {:>4} | P@{}={:.2} R@{}={:.2} RR={:.2} | {} | {}",
            query.id, status, k, p, k, r, rr, query.category, query.query
        );

        if !query.relevant_ids.is_empty() {
            total_rr += rr;
            total_p += p;
            total_r += r;
            total_hit += hit;
            evaluated += 1;
        }
    }

    if evaluated > 0 {
        let n = evaluated as f64;
        println!(
            "\n--- Aggregate ({} queries with ground truth) ---",
            evaluated
        );
        println!("  MRR:          {:.3}", total_rr / n);
        println!("  Precision@{}:  {:.3}", k, total_p / n);
        println!("  Recall@{}:     {:.3}", k, total_r / n);
        println!("  Hit Rate@{}:   {:.3}", k, total_hit / n);
    }

    Ok(())
}

pub(super) fn run_encrypt() -> Result<()> {
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

pub(super) fn run_cleanup() -> Result<()> {
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
