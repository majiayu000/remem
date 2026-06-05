use anyhow::Result;

use crate::db;
use crate::memory::format;

use super::constants::{COMPRESS_BATCH, COMPRESS_PROMPT, COMPRESS_THRESHOLD, KEEP_RECENT};

pub async fn process_compress_job(host: &str, project: &str, profile: Option<&str>) -> Result<()> {
    maybe_compress(host, project, profile).await
}

async fn maybe_compress(host: &str, project: &str, profile: Option<&str>) -> Result<()> {
    let conn = db::open_db()?;
    let total = db::count_active_observations(&conn, project)?;
    if total <= COMPRESS_THRESHOLD {
        return Ok(());
    }

    crate::log::info(
        "compress",
        &format!(
            "project={} has {} observations (threshold={}), compressing",
            project, total, COMPRESS_THRESHOLD
        ),
    );

    let old_obs = db::get_oldest_observations(&conn, project, KEEP_RECENT, COMPRESS_BATCH)?;
    if old_obs.is_empty() {
        return Ok(());
    }

    let timer = crate::log::Timer::start("compress", &format!("{} observations", old_obs.len()));
    let events = build_compress_events(&old_obs);
    let response = match crate::ai::call_ai(
        COMPRESS_PROMPT,
        &events,
        crate::ai::UsageContext {
            project: Some(project),
            operation: "compress",
            host: profile.is_none().then_some(host),
            profile,
        },
    )
    .await
    {
        Ok(value) => value,
        Err(err) => {
            crate::log::warn("compress", &format!("AI call failed: {}", err));
            timer.done(&format!("AI error: {}", err));
            return Ok(());
        }
    };

    let ids: Vec<i64> = old_obs.iter().map(|obs| obs.id).collect();
    let outcome = apply_compression_response(&conn, project, &ids, &response)?;
    match outcome {
        CompressionOutcome::Skipped {
            reason,
            source_count,
        } => {
            crate::log::info(
                "compress",
                &format!("project={project} skipped compression: {reason}"),
            );
            timer.done(&format!("{source_count} old → skipped ({reason})"));
        }
        CompressionOutcome::Compressed {
            source_count,
            replacement_count,
            marked_count,
        } => {
            timer.done(&format!(
                "{} old → {} compressed, {} marked",
                source_count, replacement_count, marked_count
            ));
        }
    }
    Ok(())
}

fn build_compress_events(old_obs: &[crate::db::models::Observation]) -> String {
    let mut events = String::from("<old_observations>\n");
    for obs in old_obs {
        events.push_str(&format!(
            "<observation type=\"{}\">\n<title>{}</title>\n<subtitle>{}</subtitle>\n<narrative>{}</narrative>\n</observation>\n",
            format::xml_escape_attr(&obs.r#type),
            format::xml_escape_text(obs.title.as_deref().unwrap_or("")),
            format::xml_escape_text(obs.subtitle.as_deref().unwrap_or("")),
            format::xml_escape_text(obs.narrative.as_deref().unwrap_or("")),
        ));
    }
    events.push_str("</old_observations>");
    events
}

fn store_compressed_observations(
    conn: &rusqlite::Connection,
    project: &str,
    response: &str,
    compressed: &[format::ParsedObservation],
) -> Result<()> {
    let memory_session_id = format!("compressed-{}", chrono::Utc::now().timestamp());
    let usage = response.len() as i64 / 4;

    for obs in compressed {
        let facts_json = if obs.facts.is_empty() {
            None
        } else {
            Some(serde_json::to_string(&obs.facts)?)
        };
        let concepts_json = if obs.concepts.is_empty() {
            None
        } else {
            Some(serde_json::to_string(&obs.concepts)?)
        };
        db::insert_observation(
            conn,
            &memory_session_id,
            project,
            &obs.obs_type,
            obs.title.as_deref(),
            obs.subtitle.as_deref(),
            obs.narrative.as_deref(),
            facts_json.as_deref(),
            concepts_json.as_deref(),
            None,
            None,
            None,
            usage / compressed.len().max(1) as i64,
        )?;
    }

    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CompressionOutcome {
    Skipped {
        reason: &'static str,
        source_count: usize,
    },
    Compressed {
        source_count: usize,
        replacement_count: usize,
        marked_count: usize,
    },
}

fn apply_compression_response(
    conn: &rusqlite::Connection,
    project: &str,
    source_ids: &[i64],
    response: &str,
) -> Result<CompressionOutcome> {
    let compressed = format::parse_observations(response);
    if compressed.is_empty() {
        return Ok(CompressionOutcome::Skipped {
            reason: "no replacement observations parsed",
            source_count: source_ids.len(),
        });
    }

    with_compression_savepoint(conn, || {
        store_compressed_observations(conn, project, response, &compressed)?;
        let marked = db::mark_observations_compressed(conn, source_ids)?;
        Ok(CompressionOutcome::Compressed {
            source_count: source_ids.len(),
            replacement_count: compressed.len(),
            marked_count: marked,
        })
    })
}

fn with_compression_savepoint<T>(
    conn: &rusqlite::Connection,
    f: impl FnOnce() -> Result<T>,
) -> Result<T> {
    conn.execute_batch("SAVEPOINT remem_compression_apply;")?;
    match f() {
        Ok(value) => {
            conn.execute_batch("RELEASE SAVEPOINT remem_compression_apply;")?;
            Ok(value)
        }
        Err(error) => {
            if let Err(rollback_error) = conn.execute_batch(
                "ROLLBACK TO SAVEPOINT remem_compression_apply;
                 RELEASE SAVEPOINT remem_compression_apply;",
            ) {
                return Err(error.context(format!(
                    "compression rollback also failed: {rollback_error}"
                )));
            }
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::{params, Connection};

    fn setup_observation_schema(conn: &Connection) -> Result<()> {
        conn.execute_batch(
            "CREATE TABLE observations (
                id INTEGER PRIMARY KEY,
                memory_session_id TEXT NOT NULL,
                project TEXT,
                type TEXT NOT NULL,
                title TEXT,
                subtitle TEXT,
                narrative TEXT,
                facts TEXT,
                concepts TEXT,
                files_read TEXT,
                files_modified TEXT,
                prompt_number INTEGER,
                created_at TEXT,
                created_at_epoch INTEGER,
                discovery_tokens INTEGER DEFAULT 0,
                status TEXT DEFAULT 'active',
                last_accessed_epoch INTEGER,
                branch TEXT,
                commit_sha TEXT
            );",
        )?;
        Ok(())
    }

    fn insert_source_observation(conn: &Connection, status: &str) -> Result<i64> {
        let id = db::insert_observation(
            conn,
            "source-session",
            "proj",
            "discovery",
            Some("Source"),
            None,
            Some("Source observation"),
            None,
            None,
            None,
            None,
            None,
            0,
        )?;
        conn.execute(
            "UPDATE observations SET status = ?1 WHERE id = ?2",
            params![status, id],
        )?;
        Ok(id)
    }

    fn statuses_for(conn: &Connection, ids: &[i64]) -> Result<Vec<String>> {
        let placeholders = ids
            .iter()
            .enumerate()
            .map(|(idx, _)| format!("?{}", idx + 1))
            .collect::<Vec<_>>()
            .join(", ");
        let sql =
            format!("SELECT status FROM observations WHERE id IN ({placeholders}) ORDER BY id");
        let params = ids
            .iter()
            .map(|id| Box::new(*id) as Box<dyn rusqlite::types::ToSql>)
            .collect::<Vec<_>>();
        let refs = crate::db::to_sql_refs(&params);
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(refs.as_slice(), |row| row.get::<_, String>(0))?;
        crate::db::query::collect_rows(rows)
    }

    fn compressed_titles(conn: &Connection) -> Result<Vec<String>> {
        let mut stmt = conn.prepare(
            "SELECT COALESCE(title, '')
             FROM observations
             WHERE memory_session_id LIKE 'compressed-%'
             ORDER BY id",
        )?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        crate::db::query::collect_rows(rows)
    }

    fn valid_response(title: &str) -> String {
        format!(
            "<observation>
                <type>decision</type>
                <title>{title}</title>
                <narrative>Compressed narrative</narrative>
             </observation>"
        )
    }

    #[test]
    fn empty_compression_response_leaves_sources_active() -> Result<()> {
        let conn = Connection::open_in_memory()?;
        setup_observation_schema(&conn)?;
        let ids = vec![
            insert_source_observation(&conn, "active")?,
            insert_source_observation(&conn, "stale")?,
        ];

        let outcome = apply_compression_response(&conn, "proj", &ids, "")?;

        assert_eq!(
            outcome,
            CompressionOutcome::Skipped {
                reason: "no replacement observations parsed",
                source_count: 2,
            }
        );
        assert_eq!(statuses_for(&conn, &ids)?, vec!["active", "stale"]);
        assert!(compressed_titles(&conn)?.is_empty());
        Ok(())
    }

    #[test]
    fn malformed_compression_response_leaves_sources_active() -> Result<()> {
        let conn = Connection::open_in_memory()?;
        setup_observation_schema(&conn)?;
        let ids = vec![insert_source_observation(&conn, "active")?];

        let outcome = apply_compression_response(
            &conn,
            "proj",
            &ids,
            "<observation><type>decision</type><title>broken",
        )?;

        assert_eq!(
            outcome,
            CompressionOutcome::Skipped {
                reason: "no replacement observations parsed",
                source_count: 1,
            }
        );
        assert_eq!(statuses_for(&conn, &ids)?, vec!["active"]);
        assert!(compressed_titles(&conn)?.is_empty());
        Ok(())
    }

    #[test]
    fn valid_compression_inserts_replacement_then_marks_sources() -> Result<()> {
        let conn = Connection::open_in_memory()?;
        setup_observation_schema(&conn)?;
        let ids = vec![
            insert_source_observation(&conn, "active")?,
            insert_source_observation(&conn, "stale")?,
        ];

        let outcome =
            apply_compression_response(&conn, "proj", &ids, &valid_response("Compressed"))?;

        assert_eq!(
            outcome,
            CompressionOutcome::Compressed {
                source_count: 2,
                replacement_count: 1,
                marked_count: 2,
            }
        );
        assert_eq!(statuses_for(&conn, &ids)?, vec!["compressed", "compressed"]);
        assert_eq!(compressed_titles(&conn)?, vec!["Compressed"]);
        Ok(())
    }

    #[test]
    fn replacement_insert_failure_rolls_back_sources_and_replacements() -> Result<()> {
        let conn = Connection::open_in_memory()?;
        setup_observation_schema(&conn)?;
        conn.execute_batch(
            "CREATE TRIGGER fail_bad_compression
             BEFORE INSERT ON observations
             WHEN NEW.memory_session_id LIKE 'compressed-%'
                  AND NEW.title = 'Bad replacement'
             BEGIN
                 SELECT RAISE(FAIL, 'bad compressed insert');
             END;",
        )?;
        let ids = vec![
            insert_source_observation(&conn, "active")?,
            insert_source_observation(&conn, "stale")?,
        ];
        let response = format!(
            "{}\n{}",
            valid_response("Good replacement"),
            valid_response("Bad replacement")
        );

        let error = apply_compression_response(&conn, "proj", &ids, &response)
            .expect_err("insert trigger should fail");

        assert!(error.to_string().contains("bad compressed insert"));
        assert_eq!(statuses_for(&conn, &ids)?, vec!["active", "stale"]);
        assert!(compressed_titles(&conn)?.is_empty());
        Ok(())
    }
}
