use anyhow::Result;

use crate::db;
use crate::memory_format;

use super::constants::{COMPRESS_BATCH, COMPRESS_PROMPT, COMPRESS_THRESHOLD, KEEP_RECENT};

pub async fn process_compress_job(project: &str) -> Result<()> {
    maybe_compress(project).await
}

async fn maybe_compress(project: &str) -> Result<()> {
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

    let compressed = memory_format::parse_observations(&response);
    if !compressed.is_empty() {
        store_compressed_observations(&conn, project, &response, &compressed)?;
    }

    let ids: Vec<i64> = old_obs.iter().map(|obs| obs.id).collect();
    let marked = db::mark_observations_compressed(&conn, &ids)?;
    timer.done(&format!(
        "{} old → {} compressed, {} marked",
        old_obs.len(),
        compressed.len(),
        marked
    ));
    Ok(())
}

fn build_compress_events(old_obs: &[crate::db_models::Observation]) -> String {
    let mut events = String::from("<old_observations>\n");
    for obs in old_obs {
        events.push_str(&format!(
            "<observation type=\"{}\">\n<title>{}</title>\n<subtitle>{}</subtitle>\n<narrative>{}</narrative>\n</observation>\n",
            memory_format::xml_escape_attr(&obs.r#type),
            memory_format::xml_escape_text(obs.title.as_deref().unwrap_or("")),
            memory_format::xml_escape_text(obs.subtitle.as_deref().unwrap_or("")),
            memory_format::xml_escape_text(obs.narrative.as_deref().unwrap_or("")),
        ));
    }
    events.push_str("</old_observations>");
    events
}

fn store_compressed_observations(
    conn: &rusqlite::Connection,
    project: &str,
    response: &str,
    compressed: &[memory_format::ParsedObservation],
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
