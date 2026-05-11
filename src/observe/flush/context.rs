use anyhow::Result;

use crate::db;
use crate::memory_format::{xml_escape_attr, xml_escape_text, OBSERVATION_TYPES};

pub(crate) fn build_existing_context(conn: &rusqlite::Connection, project: &str) -> Result<String> {
    let recent_obs = db::query_observations(conn, project, OBSERVATION_TYPES, 10)?;
    let recent_mem = crate::memory::list_memories(conn, project, None, 10, 0, false, None)?;

    if recent_obs.is_empty() && recent_mem.is_empty() {
        return Ok(String::new());
    }

    let mut buf = String::from("<existing_memories>\n");

    for obs in &recent_obs {
        let title_attr = obs
            .title
            .as_deref()
            .map(|title| format!(" title=\"{}\"", xml_escape_attr(title)))
            .unwrap_or_default();
        let body = obs
            .subtitle
            .as_deref()
            .map(xml_escape_text)
            .unwrap_or_default();
        buf.push_str(&format!(
            "<memory type=\"{}\" source=\"observation\"{}>{}</memory>\n",
            xml_escape_attr(&obs.r#type),
            title_attr,
            body,
        ));
    }

    for mem in &recent_mem {
        let preview = mem
            .text
            .lines()
            .next()
            .map(|line| db::truncate_str(line, 120))
            .unwrap_or("");
        buf.push_str(&format!(
            "<memory type=\"{}\" source=\"memory\" title=\"{}\">{}</memory>\n",
            xml_escape_attr(&mem.memory_type),
            xml_escape_attr(&mem.title),
            xml_escape_text(preview),
        ));
    }

    buf.push_str("</existing_memories>\n");
    Ok(buf)
}

pub(crate) fn build_session_events_xml(batch: &[db::PendingObservation]) -> String {
    let mut events = String::new();
    for (index, pending) in batch.iter().enumerate() {
        events.push_str(&format!(
            "<event index=\"{}\">\n\
             <tool>{}</tool>\n\
             <working_directory>{}</working_directory>\n\
             <parameters>{}</parameters>\n\
             <outcome>{}</outcome>\n\
             </event>\n",
            index + 1,
            xml_escape_text(&pending.tool_name),
            xml_escape_text(pending.cwd.as_deref().unwrap_or(".")),
            xml_escape_text(pending.tool_input.as_deref().unwrap_or("")),
            xml_escape_text(pending.tool_response.as_deref().unwrap_or("")),
        ));
    }
    events
}

#[cfg(test)]
mod tests;
