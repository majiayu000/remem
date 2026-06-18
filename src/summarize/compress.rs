use anyhow::Result;

use crate::db;
use crate::memory::format;

use super::constants::{COMPRESS_BATCH, COMPRESS_PROMPT, COMPRESS_THRESHOLD, KEEP_RECENT};

const NO_REPLACEMENTS_REASON: &str = "no replacement observations parsed";
const INVALID_REPLACEMENTS_REASON: &str = "invalid replacement observations parsed";

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
            session_id: None,
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
            return Err(err);
        }
    };

    let outcome = apply_compression_response(&conn, project, &old_obs, &response)?;
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
) -> Result<StoredCompressedObservations> {
    let memory_session_id = format!("compressed-{}", chrono::Utc::now().timestamp());
    let usage = response.len() as i64 / 4;
    let mut ids = Vec::with_capacity(compressed.len());

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
        let id = db::insert_observation(
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
        ids.push(id);
    }

    Ok(StoredCompressedObservations {
        ids,
        memory_session_id,
    })
}

struct StoredCompressedObservations {
    ids: Vec<i64>,
    memory_session_id: String,
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
    source_observations: &[db::models::Observation],
    response: &str,
) -> Result<CompressionOutcome> {
    let compressed = format::parse_observations(response);
    if compressed.is_empty() {
        return Ok(CompressionOutcome::Skipped {
            reason: NO_REPLACEMENTS_REASON,
            source_count: source_observations.len(),
        });
    }
    if compressed.iter().any(|obs| !has_replacement_content(obs)) {
        return Ok(CompressionOutcome::Skipped {
            reason: INVALID_REPLACEMENTS_REASON,
            source_count: source_observations.len(),
        });
    }

    let source_ids: Vec<i64> = source_observations.iter().map(|obs| obs.id).collect();
    with_compression_savepoint(conn, || {
        let stored = store_compressed_observations(conn, project, response, &compressed)?;
        let linked = db::insert_compressed_observation_sources(
            conn,
            &stored.ids,
            source_observations,
            &stored.memory_session_id,
        )?;
        let expected_links = stored.ids.len() * source_observations.len();
        if linked != expected_links {
            anyhow::bail!("inserted {linked} of {expected_links} compressed source links");
        }
        let marked = db::mark_observations_compressed(conn, &source_ids)?;
        if marked != source_ids.len() {
            anyhow::bail!(
                "marked {marked} of {} source observations compressed",
                source_ids.len()
            );
        }
        Ok(CompressionOutcome::Compressed {
            source_count: source_ids.len(),
            replacement_count: compressed.len(),
            marked_count: marked,
        })
    })
}

fn has_replacement_content(obs: &format::ParsedObservation) -> bool {
    has_text(obs.title.as_deref())
        || has_text(obs.subtitle.as_deref())
        || has_text(obs.narrative.as_deref())
        || !obs.facts.is_empty()
        || !obs.concepts.is_empty()
}

fn has_text(value: Option<&str>) -> bool {
    value.is_some_and(|value| !value.trim().is_empty())
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
mod tests;
