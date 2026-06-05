use anyhow::Result;

use crate::db;
use crate::memory::format;

use super::constants::{COMPRESS_BATCH, COMPRESS_PROMPT, COMPRESS_THRESHOLD, KEEP_RECENT};

pub async fn process_compress_job(project: &str) -> Result<()> {
    maybe_compress(project).await
}

async fn maybe_compress(project: &str) -> Result<()> {
    let mut conn = db::open_db()?;
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

    let outcome = apply_compression_response(&mut conn, project, &old_obs, &response)?;
    timer.done(&format!(
        "{} old → {} compressed, {} marked",
        old_obs.len(),
        outcome.compressed_count,
        outcome.marked_count
    ));
    Ok(())
}

struct CompressionOutcome {
    compressed_count: usize,
    marked_count: usize,
}

fn apply_compression_response(
    conn: &mut rusqlite::Connection,
    project: &str,
    old_obs: &[crate::db::models::Observation],
    response: &str,
) -> Result<CompressionOutcome> {
    let compressed = format::parse_observations(response);
    if compressed.is_empty() {
        crate::log::warn(
            "compress",
            &format!(
                "compression_skipped project={} reason=no_replacement_observations",
                project
            ),
        );
        return Ok(CompressionOutcome {
            compressed_count: 0,
            marked_count: 0,
        });
    }

    let ids: Vec<i64> = old_obs.iter().map(|obs| obs.id).collect();
    let marked =
        store_compressed_observations_and_mark_sources(conn, project, response, &compressed, &ids)
            .map_err(|err| {
                crate::log::error(
                    "compress",
                    &format!(
                        "compression_failed project={} reason=replacement_write_or_source_mark_failed error={}",
                        project, err
                    ),
                );
                err
            })?;
    Ok(CompressionOutcome {
        compressed_count: compressed.len(),
        marked_count: marked,
    })
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

fn store_compressed_observations_and_mark_sources(
    conn: &mut rusqlite::Connection,
    project: &str,
    response: &str,
    compressed: &[format::ParsedObservation],
    source_ids: &[i64],
) -> Result<usize> {
    let tx = conn.transaction()?;
    store_compressed_observations(&tx, project, response, compressed)?;
    let marked = db::mark_observations_compressed(&tx, source_ids)?;
    tx.commit()?;
    Ok(marked)
}

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use rusqlite::Connection;

    use super::*;

    const PROJECT: &str = "compress-test";

    fn setup_conn() -> Result<Connection> {
        let conn = Connection::open_in_memory()?;
        crate::migrate::run_migrations(&conn)?;
        Ok(conn)
    }

    fn insert_source_observation(conn: &Connection, title: &str) -> Result<i64> {
        db::insert_observation(
            conn,
            "source-session",
            PROJECT,
            "discovery",
            Some(title),
            None,
            Some("source narrative"),
            None,
            None,
            None,
            None,
            None,
            1,
        )
    }

    fn observation_status(conn: &Connection, id: i64) -> Result<String> {
        Ok(conn.query_row(
            "SELECT status FROM observations WHERE id = ?1",
            [id],
            |row| row.get(0),
        )?)
    }

    fn observation_count(conn: &Connection) -> Result<i64> {
        Ok(conn.query_row("SELECT COUNT(*) FROM observations", [], |row| row.get(0))?)
    }

    fn old_obs(conn: &Connection, id: i64) -> Result<Vec<crate::db::models::Observation>> {
        let rows = db::get_observations_by_ids(conn, &[id], Some(PROJECT))?;
        anyhow::ensure!(
            rows.len() == 1,
            "source observation {id} should exist exactly once"
        );
        Ok(rows)
    }

    #[test]
    fn empty_ai_response_leaves_sources_active() -> Result<()> {
        let mut conn = setup_conn()?;
        let source_id = insert_source_observation(&conn, "source")?;
        let sources = old_obs(&conn, source_id)?;

        let outcome = apply_compression_response(&mut conn, PROJECT, &sources, "")?;

        assert_eq!(outcome.compressed_count, 0);
        assert_eq!(outcome.marked_count, 0);
        assert_eq!(observation_status(&conn, source_id)?, "active");
        assert_eq!(observation_count(&conn)?, 1);
        Ok(())
    }

    #[test]
    fn malformed_ai_response_leaves_sources_active() -> Result<()> {
        let mut conn = setup_conn()?;
        let source_id = insert_source_observation(&conn, "source")?;
        let sources = old_obs(&conn, source_id)?;

        let outcome = apply_compression_response(
            &mut conn,
            PROJECT,
            &sources,
            "<observation><title>truncated",
        )?;

        assert_eq!(outcome.compressed_count, 0);
        assert_eq!(outcome.marked_count, 0);
        assert_eq!(observation_status(&conn, source_id)?, "active");
        assert_eq!(observation_count(&conn)?, 1);
        Ok(())
    }

    #[test]
    fn valid_compression_inserts_replacements_and_marks_sources() -> Result<()> {
        let mut conn = setup_conn()?;
        let source_id = insert_source_observation(&conn, "source")?;
        let sources = old_obs(&conn, source_id)?;
        let response = r#"
<observation>
<type>discovery</type>
<title>Compressed insight</title>
<narrative>Condensed source observations.</narrative>
</observation>
"#;

        let outcome = apply_compression_response(&mut conn, PROJECT, &sources, response)?;

        assert_eq!(outcome.compressed_count, 1);
        assert_eq!(outcome.marked_count, 1);
        assert_eq!(observation_status(&conn, source_id)?, "compressed");
        assert_eq!(observation_count(&conn)?, 2);
        Ok(())
    }

    #[test]
    fn replacement_insert_failure_rolls_back_source_status_changes() -> Result<()> {
        let mut conn = setup_conn()?;
        let source_id = insert_source_observation(&conn, "source")?;
        let sources = old_obs(&conn, source_id)?;
        conn.execute_batch(
            "CREATE TRIGGER block_compressed_insert
             BEFORE INSERT ON observations
             WHEN NEW.memory_session_id LIKE 'compressed-%'
             BEGIN
                 SELECT RAISE(ABORT, 'blocked replacement insert');
             END;",
        )?;
        let response = r#"
<observation>
<type>discovery</type>
<title>Compressed insight</title>
<narrative>Condensed source observations.</narrative>
</observation>
"#;

        let err = match apply_compression_response(&mut conn, PROJECT, &sources, response) {
            Ok(_) => anyhow::bail!("replacement insert failure should be reported"),
            Err(err) => err,
        };

        assert!(
            err.to_string().contains("blocked replacement insert"),
            "unexpected error: {err}"
        );
        assert_eq!(observation_status(&conn, source_id)?, "active");
        assert_eq!(observation_count(&conn)?, 1);
        Ok(())
    }
}
