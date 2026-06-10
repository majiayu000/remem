use anyhow::Result;

use crate::{
    db,
    memory::service::{self, CurrentStateRequest, CurrentStateResult},
};

use super::show::format_memory_timestamp;

#[allow(clippy::too_many_arguments)]
pub(in crate::cli) fn run_current_state(
    state_key: &str,
    project: Option<&str>,
    owner_scope: Option<&str>,
    owner_key: Option<&str>,
    memory_type: Option<&str>,
    as_of_epoch: Option<i64>,
    json: bool,
) -> Result<()> {
    let conn = db::open_db()?;
    let request = CurrentStateRequest {
        state_key: state_key.to_string(),
        project: project.map(str::to_string),
        owner_scope: owner_scope.map(str::to_string),
        owner_key: owner_key.map(str::to_string),
        memory_type: memory_type.map(str::to_string),
        as_of_epoch,
        include_history: true,
    };
    let result = service::current_state(&conn, &request)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        print!("{}", render_current_state(&result));
    }
    Ok(())
}

pub(super) fn render_current_state(result: &CurrentStateResult) -> String {
    let mut output = String::new();
    output.push_str(&format!("Current state: {}\n", result.status));
    output.push_str(&format!("  state_key: {}\n", result.state_key));
    if let Some(as_of_epoch) = result.as_of_epoch {
        output.push_str(&format!(
            "  as_of: {}\n",
            format_memory_timestamp(as_of_epoch)
        ));
    }
    if let Some(state) = &result.state {
        output.push_str(&format!(
            "  owner: {}/{} type={}\n",
            state.owner_scope, state.owner_key, state.memory_type
        ));
    }

    if !result.matches.is_empty() {
        output.push_str("\nMatches:\n");
        for state in &result.matches {
            output.push_str(&format!(
                "  state_key_id={} owner={}/{} type={} current={:?}\n",
                state.id,
                state.owner_scope,
                state.owner_key,
                state.memory_type,
                state.current_memory_id
            ));
        }
        output.push_str("\nRetry with --owner-scope/--owner-key or --type to disambiguate.\n");
        return output;
    }

    if let Some(current) = &result.current {
        output.push_str("\nAnswer:\n");
        output.push_str(&format!(
            "  [#{}] {} | {} | updated {}\n",
            current.id,
            current.title,
            current.status,
            format_memory_timestamp(current.updated_at_epoch)
        ));
        output.push_str(&format!("  {}\n", preview_current_text(&current.text)));
    }

    if !result.conflicts.is_empty() {
        output.push_str("\nUnresolved conflicts:\n");
        for memory in &result.conflicts {
            output.push_str(&format_memory_ref(memory));
        }
    }

    if !result.history.is_empty() {
        output.push_str("\nHistory:\n");
        for memory in &result.history {
            output.push_str(&format_memory_ref(memory));
        }
    }

    if !result.facts.is_empty() {
        output.push_str("\nFacts:\n");
        for fact in &result.facts {
            output.push_str(&format!(
                "  fact#{} {} {} {:?}-{:?} status={}\n",
                fact.id,
                fact.predicate,
                fact.object,
                fact.valid_from_epoch,
                fact.valid_to_epoch,
                fact.status
            ));
        }
    }

    if !result.why.is_empty() {
        output.push_str("\nWhy:\n");
        for edge in &result.why {
            output.push_str(&format!(
                "  {} {:?}->{:?}",
                edge.edge_type, edge.from_memory_id, edge.to_memory_id
            ));
            if let Some(reason) = &edge.reason {
                output.push_str(&format!(" reason={reason}"));
            }
            if !edge.evidence_event_ids.is_empty() {
                output.push_str(&format!(" evidence={:?}", edge.evidence_event_ids));
            }
            output.push('\n');
        }
    }

    output
}

fn format_memory_ref(memory: &crate::memory::current_state::CurrentStateMemoryRef) -> String {
    let mut line = format!(
        "  [#{}] {} | {} | updated {}",
        memory.id,
        memory.title,
        memory.status,
        format_memory_timestamp(memory.updated_at_epoch)
    );
    if let Some(relation) = &memory.relation {
        line.push_str(&format!(" via {relation}"));
    }
    if let Some(reason) = &memory.reason {
        line.push_str(&format!(" reason={reason}"));
    }
    if !memory.evidence_event_ids.is_empty() {
        line.push_str(&format!(" evidence={:?}", memory.evidence_event_ids));
    }
    line.push('\n');
    line
}

fn preview_current_text(text: &str) -> String {
    let first_line = text.lines().next().unwrap_or_default();
    first_line.chars().take(160).collect()
}
